use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use p9ime::common::{to_wide, DLL_FILENAME, IME_NAME, INSTALL_DIR_NAME, UNINSTALLER_FILENAME};
use p9ime::files::{cancel_pending_reboot_deletes, replace_file, sweep_stale_files};
use p9ime::registration::{register_all_ime, register_uninstaller};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK,
};
use windows_core::PCWSTR;

fn show_message(title: &str, message: &str, is_error: bool, quiet: bool) {
    if is_error {
        eprintln!("{}: {}", title, message);
    } else {
        println!("{}: {}", title, message);
    }
    if !quiet {
        let title_w = to_wide(title);
        let msg_w = to_wide(message);
        let flags = if is_error {
            MB_OK | MB_ICONERROR
        } else {
            MB_OK | MB_ICONINFORMATION
        };
        unsafe {
            MessageBoxW(
                HWND::default(),
                PCWSTR(msg_w.as_ptr()),
                PCWSTR(title_w.as_ptr()),
                flags,
            );
        }
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            fs::copy(entry.path(), dest_path)?;
        }
    }
    Ok(())
}

fn get_install_dir() -> PathBuf {
    let program_files =
        env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string());
    PathBuf::from(program_files).join(INSTALL_DIR_NAME)
}

fn run_install(quiet: bool) -> Result<(), String> {
    let exe_path =
        env::current_exe().map_err(|e| format!("Failed to locate installer executable: {}", e))?;
    let source_dir = exe_path
        .parent()
        .ok_or_else(|| "Failed to determine installer directory.".to_string())?;

    let src_dll = source_dir.join(DLL_FILENAME);
    let src_uninstaller = source_dir.join(UNINSTALLER_FILENAME);

    if !src_dll.exists() {
        return Err(format!(
            "Required file '{}' not found alongside installer at:\n{}",
            DLL_FILENAME,
            src_dll.display()
        ));
    }
    if !src_uninstaller.exists() {
        return Err(format!(
            "Required file '{}' not found alongside installer at:\n{}",
            UNINSTALLER_FILENAME,
            src_uninstaller.display()
        ));
    }

    let install_dir = get_install_dir();
    fs::create_dir_all(&install_dir).map_err(|e| {
        format!(
            "Failed to create directory '{}' (Please run as Administrator): {}",
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
    // Clean up `*.old` leftovers from earlier in-use replacements.
    sweep_stale_files(&install_dir, false);

    // replace_file() handles the case where the existing DLL/EXE is loaded by
    // a running process (Windows refuses to overwrite it): the old file is
    // renamed aside and removed at the next reboot.
    let dll_was_in_use = replace_file(&src_dll, &dst_dll).map_err(|e| {
        format!(
            "Failed to install '{}' to '{}': {}",
            src_dll.display(),
            dst_dll.display(),
            e
        )
    })?;

    let uninstaller_was_in_use = replace_file(&src_uninstaller, &dst_uninstaller).map_err(|e| {
        format!(
            "Failed to install '{}' to '{}': {}",
            src_uninstaller.display(),
            dst_uninstaller.display(),
            e
        )
    })?;
    let replaced_in_use = dll_was_in_use || uninstaller_was_in_use;

    // Copy optional "components" folder if present alongside installer.exe
    let optional_components_src = source_dir.join("components");
    if optional_components_src.is_dir() {
        let optional_components_dst = install_dir.join("components");
        copy_dir_recursive(&optional_components_src, &optional_components_dst).map_err(|e| {
            format!(
                "Failed to copy additional components to '{}': {}",
                optional_components_dst.display(),
                e
            )
        })?;
    }

    let install_dir_str = install_dir.to_string_lossy().into_owned();
    let dst_dll_str = dst_dll.to_string_lossy().into_owned();
    let dst_uninstaller_str = dst_uninstaller.to_string_lossy().into_owned();

    // Register COM InprocServer32 + TSF Profile & Categories
    register_all_ime(&dst_dll_str).map_err(|e| {
        format!(
            "Failed to register TSF IME with Windows (ensure installer is run as Administrator): {}",
            e
        )
    })?;

    // Register Uninstaller in Windows "Installed Apps" / "Programs and Features"
    register_uninstaller(&install_dir_str, &dst_uninstaller_str, &dst_dll_str)
        .map_err(|e| format!("Failed to register uninstaller in Windows Registry: {}", e))?;

    let mut msg = format!(
        "Successfully installed {} to:\n{}\n\nRegistered TSF IME and Windows Uninstaller.",
        IME_NAME, install_dir_str
    );
    if replaced_in_use {
        msg.push_str(
            "\n\nA previous version was in use by running applications. \
             They will keep using it until they are restarted; \
             the old files are removed automatically at the next reboot.",
        );
    }
    show_message(IME_NAME, &msg, false, quiet);

    Ok(())
}

fn main() -> ExitCode {
    let quiet = env::args().any(|a| a == "--quiet" || a == "/S" || a == "/quiet");

    match run_install(quiet) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            show_message(&format!("{} Installer Error", IME_NAME), &err, true, quiet);
            ExitCode::FAILURE
        }
    }
}
