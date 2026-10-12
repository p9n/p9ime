//! File-replacement helpers that are safe against the Windows "image in use"
//! rules and against uninstall → reinstall without a reboot.
//!
//! Windows will not let you delete or overwrite an EXE/DLL that is currently
//! mapped into a process, but it *will* let you rename it. Everything here is
//! built on that: when a canonical path (`p9ime.dll`, `uninstaller.exe`) is
//! busy we move it aside to `<name>.<pid>.old` and only ever target the
//! renamed file for deferred deletion, so a later reinstall that recreates the
//! canonical path is never collateral damage.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, REG_MULTI_SZ, REG_VALUE_TYPE,
};
use windows_core::PCWSTR;

use crate::common::to_wide;

/// Suffix given to files that were renamed aside because they were in use.
pub const STALE_SUFFIX: &str = ".old";
/// Suffix used while staging a new file next to its destination.
pub const TEMP_SUFFIX: &str = ".tmp";

const SESSION_MANAGER_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager";
/// Windows spills into the `2` variant when the first value grows large.
const PENDING_VALUES: [&str; 2] = [
    "PendingFileRenameOperations",
    "PendingFileRenameOperations2",
];

/// Ask Windows to delete `path` on the next reboot. Entries whose target no
/// longer exists at boot are ignored, so this is safe to pair with an
/// immediate deletion attempt. Requires administrator rights.
pub fn schedule_delete_on_reboot(path: &Path) {
    let wide = to_wide(&path.to_string_lossy());
    unsafe {
        let _ = MoveFileExW(
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            MOVEFILE_DELAY_UNTIL_REBOOT,
        );
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path.file_name().unwrap_or_default());
    name.push(format!(".{}{}", std::process::id(), suffix));
    path.with_file_name(name)
}

/// Name used when moving `path` aside: `<file>.<pid>.old`.
pub fn stale_name(path: &Path) -> PathBuf {
    with_suffix(path, STALE_SUFFIX)
}

/// Rename `path` out of the way so the canonical name becomes free. Works even
/// when `path` is a running EXE or a loaded DLL.
pub fn rename_aside(path: &Path) -> io::Result<PathBuf> {
    let stale = stale_name(path);
    fs::rename(path, &stale)?;
    Ok(stale)
}

/// Move an already-staged sibling file into place at `dst`, replacing whatever
/// is there. A plain rename is atomic; if Windows refuses because the existing
/// file is a mapped image, that file is renamed aside and queued for deletion
/// at the next reboot. Returns `true` in that case.
fn move_into_place(staged: &Path, dst: &Path) -> io::Result<bool> {
    let first_err = match fs::rename(staged, dst) {
        Ok(()) => return Ok(false),
        Err(e) => e,
    };
    if !dst.exists() {
        let _ = fs::remove_file(staged);
        return Err(first_err);
    }

    let stale = match rename_aside(dst) {
        Ok(p) => p,
        Err(e) => {
            let _ = fs::remove_file(staged);
            return Err(io::Error::new(
                e.kind(),
                format!(
                    "{} is in use and could not be moved aside: {} (original error: {})",
                    dst.display(),
                    e,
                    first_err
                ),
            ));
        }
    };
    schedule_delete_on_reboot(&stale);

    match fs::rename(staged, dst) {
        Ok(()) => Ok(true),
        Err(e) => {
            let _ = fs::remove_file(staged);
            Err(e)
        }
    }
}

/// Write `bytes` to `dst`, replacing `dst` even if it is currently loaded.
/// Returns `true` if the old file had to be moved aside (a reboot will finish
/// the cleanup).
pub fn install_bytes(bytes: &[u8], dst: &Path) -> io::Result<bool> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    let staged = with_suffix(dst, TEMP_SUFFIX);
    fs::write(&staged, bytes)?;
    move_into_place(&staged, dst)
}

/// Copy `src` to `dst`, replacing `dst` even if it is currently loaded.
/// Returns `true` if the old file had to be moved aside (a reboot will finish
/// the cleanup).
pub fn install_file(src: &Path, dst: &Path) -> io::Result<bool> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    let staged = with_suffix(dst, TEMP_SUFFIX);
    fs::copy(src, &staged)?;
    move_into_place(&staged, dst)
}

/// Delete leftover `*.old` / `*.tmp` files in `dir`. Files still in use are
/// left alone; if `schedule_if_busy` is set they are queued for deletion at
/// reboot.
pub fn sweep_stale_files(dir: &Path, schedule_if_busy: bool) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_stale = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(STALE_SUFFIX) || n.ends_with(TEMP_SUFFIX));
        if is_stale && path.is_file() && fs::remove_file(&path).is_err() && schedule_if_busy {
            schedule_delete_on_reboot(&path);
        }
    }
}

/// Remove any reboot-time delete/rename that a previous uninstall queued for
/// exactly these paths, so a reinstall before rebooting is not wiped out at
/// the next boot. Entries for other paths (including other software) are
/// preserved verbatim.
pub fn cancel_pending_reboot_deletes(paths: &[&Path]) {
    let targets: Vec<String> = paths
        .iter()
        .map(|p| normalize(&p.to_string_lossy()))
        .collect();

    unsafe {
        let key_wide = to_wide(SESSION_MANAGER_KEY);
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key_wide.as_ptr()),
            0,
            KEY_READ | KEY_SET_VALUE,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            return;
        }

        for value_name in PENDING_VALUES {
            let Some(entries) = read_multi_sz(hkey, value_name) else {
                continue;
            };

            // Entries come in (source, destination) pairs; destination "" = delete.
            let kept: Vec<Vec<u16>> = entries
                .chunks(2)
                .filter(|pair| {
                    let source = normalize(&String::from_utf16_lossy(&pair[0]));
                    !targets.contains(&source)
                })
                .flat_map(|pair| pair.to_vec())
                .collect();

            if kept.len() == entries.len() {
                continue; // nothing of ours in here
            }

            let name_wide = to_wide(value_name);
            if kept.is_empty() {
                let _ = RegDeleteValueW(hkey, PCWSTR(name_wide.as_ptr()));
            } else {
                let bytes = encode_multi_sz(&kept);
                let _ = RegSetValueExW(
                    hkey,
                    PCWSTR(name_wide.as_ptr()),
                    0,
                    REG_MULTI_SZ,
                    Some(&bytes),
                );
            }
        }

        let _ = RegCloseKey(hkey);
    }
}

/// Canonical form for comparing pending-operation paths: strip the NT
/// `\??\` / `\\?\` prefix, trailing separators, and case.
fn normalize(path: &str) -> String {
    let p = path
        .strip_prefix(r"\??\")
        .or_else(|| path.strip_prefix(r"\\?\"))
        .unwrap_or(path);
    p.trim_end_matches(['\\', '/']).to_lowercase()
}

/// Read a REG_MULTI_SZ value as a list of UTF-16 strings (without terminators).
unsafe fn read_multi_sz(hkey: HKEY, value_name: &str) -> Option<Vec<Vec<u16>>> {
    let name_wide = to_wide(value_name);
    let name = PCWSTR(name_wide.as_ptr());

    let mut kind = REG_VALUE_TYPE::default();
    let mut size: u32 = 0;
    let status = RegQueryValueExW(hkey, name, None, Some(&mut kind), None, Some(&mut size));
    if status == ERROR_FILE_NOT_FOUND || status != ERROR_SUCCESS || kind != REG_MULTI_SZ {
        return None;
    }

    let mut buf = vec![0u8; size as usize];
    let status = RegQueryValueExW(
        hkey,
        name,
        None,
        Some(&mut kind),
        Some(buf.as_mut_ptr()),
        Some(&mut size),
    );
    if status != ERROR_SUCCESS {
        return None;
    }
    buf.truncate(size as usize);

    let units: Vec<u16> = buf
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();

    // Split on NUL. A well-formed value ends "...\0\0", which yields two
    // trailing empty pieces (one for the last separator, one for the list
    // terminator). Interior empty strings are significant — they are the
    // "delete" destinations — so only those two are removed.
    let mut strings: Vec<Vec<u16>> = units.split(|&u| u == 0).map(|s| s.to_vec()).collect();
    for _ in 0..2 {
        if strings.last().is_some_and(|s| s.is_empty()) {
            strings.pop();
        }
    }
    // Entries are (source, destination) pairs. An odd count means the value
    // lacked its final terminator and we trimmed a real empty destination.
    if strings.len() % 2 == 1 {
        strings.push(Vec::new());
    }
    Some(strings)
}

/// Encode strings back into REG_MULTI_SZ bytes (each NUL-terminated, plus a
/// final NUL).
fn encode_multi_sz(strings: &[Vec<u16>]) -> Vec<u8> {
    let mut units: Vec<u16> = Vec::new();
    for s in strings {
        units.extend_from_slice(s);
        units.push(0);
    }
    units.push(0);
    units.iter().flat_map(|u| u.to_le_bytes()).collect()
}
