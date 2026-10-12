use std::env;
use std::fs;

use p9ime::common::{DLL_FILENAME, IME_NAME, UNINSTALLER_FILENAME};
use p9ime::files::{cancel_pending_reboot_deletes, install_bytes, install_file, sweep_stale_files};
use p9ime::registration::{register_all_ime, register_uninstaller};

use crate::ui::show_message;

/// The compiled IME, embedded at build time. `build.rs` points `P9IME_DLL` at
/// the cdylib produced by this same package; Cargo builds the lib before this
/// bin, so the file always exists for a real build. See the `embed-dll`
/// feature for why this is gated.
#[cfg(feature = "embed-dll")]
static DLL_PAYLOAD: &[u8] = include_bytes!(env!("P9IME_DLL"));
#[cfg(not(feature = "embed-dll"))]
static DLL_PAYLOAD: &[u8] = &[];

pub fn run(quiet: bool) -> Result<(), String> {
    if DLL_PAYLOAD.is_empty() {
        return Err(format!(
            "This installer was built without the embedded {} payload \
             (Cargo feature `embed-dll` disabled) and cannot install anything.",
            DLL_FILENAME
        ));
    }

    let self_exe =
        env::current_exe().map_err(|e| format!("Failed to locate installer executable: {}", e))?;

    let install_dir = crate::install_dir();
    fs::create_dir_all(&install_dir).map_err(|e| {
        format!(
            "Failed to create directory '{}' (please run as Administrator): {}",
            install_dir.display(),
            e
        )
    })?;

    let dst_dll = install_dir.join(DLL_FILENAME);
    let dst_uninstaller = install_dir.join(UNINSTALLER_FILENAME);

    // --- Reinstall / upgrade hygiene -------------------------------------
    // A previous uninstall may have queued reboot-time deletions for these
    // exact paths. If we are reinstalling before that reboot, those entries
    // would delete the files we are about to install. Drop them.
    cancel_pending_reboot_deletes(&[&install_dir, &dst_dll, &dst_uninstaller]);
    // Clean up `*.old` / `*.tmp` leftovers from earlier runs.
    sweep_stale_files(&install_dir, false);

    // install_* handle the case where the existing DLL/EXE is loaded by a
    // running process (Windows refuses to overwrite a mapped image): the old
    // file is renamed aside and removed at the next reboot.
    let dll_was_in_use = install_bytes(DLL_PAYLOAD, &dst_dll)
        .map_err(|e| format!("Failed to install '{}': {}", dst_dll.display(), e))?;

    // The uninstaller is this very executable.
    let uninstaller_was_in_use = install_file(&self_exe, &dst_uninstaller)
        .map_err(|e| format!("Failed to install '{}': {}", dst_uninstaller.display(), e))?;

    let install_dir_str = install_dir.to_string_lossy().into_owned();
    let dst_dll_str = dst_dll.to_string_lossy().into_owned();
    let dst_uninstaller_str = dst_uninstaller.to_string_lossy().into_owned();

    // Register COM InprocServer32 + TSF profile & categories.
    register_all_ime(&dst_dll_str).map_err(|e| {
        format!(
            "Failed to register TSF IME with Windows (ensure installer is run as Administrator): {}",
            e
        )
    })?;

    // Register in Windows "Installed apps" / "Programs and Features".
    register_uninstaller(&install_dir_str, &dst_uninstaller_str, &dst_dll_str)
        .map_err(|e| format!("Failed to register uninstaller in Windows Registry: {}", e))?;

    let mut msg = format!(
        "Successfully installed {} to:\n{}\n\nRegistered TSF IME and Windows Uninstaller.",
        IME_NAME, install_dir_str
    );
    if dll_was_in_use || uninstaller_was_in_use {
        msg.push_str(
            "\n\nA previous version was in use by running applications. \
             They will keep using it until they are restarted; \
             the old files are removed automatically at the next reboot.",
        );
    }
    show_message(IME_NAME, &msg, false, quiet);
    Ok(())
}
