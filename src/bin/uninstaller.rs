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
    unregister_uninstaller()
        .map_err(|e| format!("Failed to remove uninstaller registry entry: {}", e))?;

    // 3. Remove installed files under %ProgramFiles%\P9IME
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

    // 4. Show the result to the user *before* spawning the self-delete helper.
    //    A running executable cannot be deleted on Windows, so the helper must not
    //    fire while we are still blocked on the message box.
    let msg = if reboot_needed {
        format!(
            "{} has been unregistered.\nSome files are still in use by open applications and will be removed after the next reboot.",
            IME_NAME
        )
    } else {
        format!("{} has been completely uninstalled.", IME_NAME)
    };
    show_message(&format!("{} Uninstaller", IME_NAME), &msg, false, quiet);

    // 5. Self-delete uninstaller.exe and the installation directory.
    if let Ok(current_exe) = env::current_exe() {
        // Fallback: if the helper below fails for any reason (e.g. killed by an
        // aggressive AV), the OS removes these on the next reboot. Entries whose
        // target no longer exists are silently ignored, so this is safe to pair
        // with the immediate deletion attempt.
        schedule_delete_on_reboot(&current_exe);
        schedule_delete_on_reboot(&install_dir);

        spawn_self_delete_helper(&current_exe, &install_dir);
    }

    Ok(())
}

/// Spawn a detached `cmd.exe` that waits for this process to exit, deletes the
/// uninstaller executable, then removes the (now empty) installation directory.
fn spawn_self_delete_helper(exe: &Path, install_dir: &Path) {
    let exe_str = exe.to_string_lossy();
    let dir_str = install_dir.to_string_lossy();

    // `ping -n 2` is the classic ~1s sleep that works without a console (`timeout`
    // fails with "input redirection is not supported" under CREATE_NO_WINDOW).
    // Retry up to ~30s: the exe handle is released a moment after we return
    // from main(), and the delete fails harmlessly until then.
    let script = format!(
        "ping 127.0.0.1 -n 2 >nul & \
         for /L %i in (1,1,30) do (if exist \"{exe}\" (del /f /q \"{exe}\" >nul 2>&1 & ping 127.0.0.1 -n 2 >nul)) & \
         rmdir /q \"{dir}\" >nul 2>&1",
        exe = exe_str,
        dir = dir_str,
    );

    // Run the helper from outside the install dir: a process whose current
    // directory is `install_dir` would keep it locked and make `rmdir` fail.
    let helper_cwd = env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\"));

    // `raw_arg` is required: `Command::arg` applies MSVC-style quoting (`\"`),
    // which cmd.exe does not understand and would corrupt the quoted paths.
    let _ = Command::new("cmd.exe")
        .raw_arg("/C")
        .raw_arg(&script)
        .current_dir(helper_cwd)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
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
