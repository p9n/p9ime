use std::env;
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use p9ime::common::{to_wide, DLL_FILENAME, IME_NAME, INSTALL_DIR_NAME};
use p9ime::registration::{unregister_all_ime, unregister_uninstaller};
use windows::Win32::Foundation::HWND;
use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT};
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK,
};
use windows_core::PCWSTR;

const CREATE_NO_WINDOW: u32 = 0x08000000;

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

fn schedule_delete_on_reboot(path: &Path) {
    let wide = to_wide(&path.to_string_lossy());
    unsafe {
        let _ = MoveFileExW(
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            MOVEFILE_DELAY_UNTIL_REBOOT,
        );
    }
}

fn get_install_dir() -> PathBuf {
    let program_files =
        env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string());
    PathBuf::from(program_files).join(INSTALL_DIR_NAME)
}

fn run_uninstall(quiet: bool) -> Result<(), String> {
    // 1. Unregister TSF profile, categories, and COM InprocServer32
    unregister_all_ime().map_err(|e| {
        format!(
            "Failed to unregister TSF IME (ensure uninstaller is run as Administrator): {}",
            e
        )
    })?;

    // 2. Remove the Windows Uninstall registry entry
    unregister_uninstaller().map_err(|e| {
        format!(
            "Failed to remove uninstaller registry entry: {}",
            e
        )
    })?;

    // 3. Remove installed files under %ProgramFiles%\MinimalTsfIme
    let install_dir = get_install_dir();
    let dll_path = install_dir.join(DLL_FILENAME);
    let mut reboot_needed = false;

    if dll_path.exists() && fs::remove_file(&dll_path).is_err() {
        // An open application may still hold the in-process COM DLL loaded in memory;
        // mark it for deletion on next reboot.
        schedule_delete_on_reboot(&dll_path);
        reboot_needed = true;
    }

    let components_dir = install_dir.join("components");
    if components_dir.exists() {
        let _ = fs::remove_dir_all(&components_dir);
    }

    // 4. Schedule self-deletion of uninstaller.exe and the installation directory
    if let Ok(current_exe) = env::current_exe() {
        let exe_str = current_exe.to_string_lossy();
        let dir_str = install_dir.to_string_lossy();
        let cmd_line = format!(
            "ping 127.0.0.1 -n 3 > NUL & del /f /q \"{}\" & rmdir \"{}\"",
            exe_str, dir_str
        );
        let _ = Command::new("cmd.exe")
            .args(["/C", &cmd_line])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }

    let msg = if reboot_needed {
        format!(
            "{} has been unregistered.\nSome files are still in use by open applications and will be removed after the next reboot.",
            IME_NAME
        )
    } else {
        format!("{} has been completely uninstalled.", IME_NAME)
    };

    show_message(&format!("{} Uninstaller", IME_NAME), &msg, false, quiet);
    Ok(())
}

fn main() -> ExitCode {
    let quiet = env::args().any(|a| a == "--quiet" || a == "/S" || a == "/quiet");

    match run_uninstall(quiet) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            show_message(
                &format!("{} Uninstaller Error", IME_NAME),
                &err,
                true,
                quiet,
            );
            ExitCode::FAILURE
        }
    }
}
