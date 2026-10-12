use std::env;
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use p9ime::common::{to_wide, DLL_FILENAME, IME_NAME, INSTALL_DIR_NAME};
use p9ime::files::{rename_aside, schedule_delete_on_reboot, sweep_stale_files};
use p9ime::registration::{unregister_all_ime, unregister_uninstaller};
use windows::Win32::Foundation::HWND;
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
        // An open application still has the in-process COM DLL mapped, so it
        // cannot be deleted. It *can* be renamed: move it aside so the
        // canonical path is free for a possible reinstall, and purge the
        // renamed copy at the next reboot. Only if even the rename fails do we
        // fall back to queueing the canonical path itself.
        match rename_aside(&dll_path) {
            Ok(stale) => schedule_delete_on_reboot(&stale),
            Err(_) => schedule_delete_on_reboot(&dll_path),
        }
        reboot_needed = true;
    }

    // Leftovers from earlier in-use replacements; queue any that are busy.
    sweep_stale_files(&install_dir, true);

    let components_dir = install_dir.join("components");
    if components_dir.exists() {
        let _ = fs::remove_dir_all(&components_dir);
    }

    // 4. Move *ourselves* aside while we are still running. From here on the
    //    canonical `uninstaller.exe` path is free, so a reinstall that drops a
    //    fresh copy while our helper is still running is never touched by it.
    //    If the rename fails we fall back to deleting the canonical path.
    let self_path = env::current_exe()
        .ok()
        .map(|exe| rename_aside(&exe).unwrap_or(exe));

    // 5. Show the result to the user *before* spawning the self-delete helper.
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

    // 6. Delete the (renamed) uninstaller and the installation directory.
    if let Some(stale_exe) = self_path {
        // Reboot-time fallback in case the helper is killed (e.g. by AV).
        // Deleting a directory at boot only succeeds if it is empty, so this
        // is a harmless no-op if the user reinstalled in the meantime.
        schedule_delete_on_reboot(&stale_exe);
        schedule_delete_on_reboot(&install_dir);

        spawn_self_delete_helper(&stale_exe, &install_dir);
    }

    Ok(())
}

/// Spawn a detached `cmd.exe` that waits for this process to exit, deletes `exe`
/// (the *renamed* uninstaller, never the canonical `uninstaller.exe`), then
/// removes the installation directory if it is empty.
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
