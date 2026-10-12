use std::env;
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use p9ime::common::{DLL_FILENAME, IME_NAME, UNINSTALLER_FILENAME};
use p9ime::files::{rename_aside, schedule_delete_on_reboot, sweep_stale_files};
use p9ime::registration::{unregister_all_ime, unregister_uninstaller};

use crate::ui::show_message;

const CREATE_NO_WINDOW: u32 = 0x08000000;

pub fn run(quiet: bool) -> Result<(), String> {
    // 1. Unregister TSF profile, categories, and COM InprocServer32.
    unregister_all_ime().map_err(|e| {
        format!(
            "Failed to unregister TSF IME (ensure uninstaller is run as Administrator): {}",
            e
        )
    })?;

    // 2. Remove the Windows Uninstall registry entry.
    unregister_uninstaller()
        .map_err(|e| format!("Failed to remove uninstaller registry entry: {}", e))?;

    // 3. Remove installed files under %ProgramFiles%\P9IME.
    let install_dir = crate::install_dir();
    let mut reboot_needed = false;

    // The DLL may be mapped into running apps, in which case it can be renamed
    // but not deleted: move it aside so the canonical path is free for a
    // possible reinstall, and purge the renamed copy at the next reboot.
    reboot_needed |= remove_or_defer(&install_dir.join(DLL_FILENAME));

    // Leftovers from earlier in-use replacements; queue any that are busy.
    sweep_stale_files(&install_dir, true);

    // 4. Deal with uninstaller.exe. If *we* are that file we cannot delete
    //    ourselves while running, so rename aside now (frees the canonical
    //    name for a reinstall) and let a detached helper delete the renamed
    //    file after we exit. If we are some other copy (e.g. the downloaded
    //    installer run with --uninstall), just remove the installed one.
    let installed_uninstaller = install_dir.join(UNINSTALLER_FILENAME);
    let self_exe = env::current_exe().ok();
    let we_are_installed_copy = self_exe
        .as_deref()
        .is_some_and(|p| same_path(p, &installed_uninstaller));

    let self_delete_target = if we_are_installed_copy {
        // Fall back to the canonical path if the rename fails; the helper
        // will still be able to delete it once we have exited.
        Some(rename_aside(&installed_uninstaller).unwrap_or(installed_uninstaller))
    } else {
        reboot_needed |= remove_or_defer(&installed_uninstaller);
        None
    };

    // 5. Show the result *before* spawning the helper: a running executable
    //    cannot be deleted, so the helper must not fire while we are blocked
    //    on the message box.
    let msg = if reboot_needed {
        format!(
            "{} has been unregistered.\nSome files are still in use by open applications and will be removed after the next reboot.",
            IME_NAME
        )
    } else {
        format!("{} has been completely uninstalled.", IME_NAME)
    };
    show_message(&format!("{} Uninstaller", IME_NAME), &msg, false, quiet);

    // 6. Remove the (renamed) uninstaller and the installation directory.
    match self_delete_target {
        Some(stale_exe) => {
            // Reboot-time fallback in case the helper is killed (e.g. by AV).
            // Deleting a directory at boot only succeeds if it is empty, so
            // this is a harmless no-op if the user reinstalled in the meantime.
            schedule_delete_on_reboot(&stale_exe);
            schedule_delete_on_reboot(&install_dir);
            spawn_self_delete_helper(&stale_exe, &install_dir);
        }
        None => {
            // Nothing of ours is running from the install dir; remove it now
            // if it is empty, otherwise let the reboot fallback handle it.
            if fs::remove_dir(&install_dir).is_err() && install_dir.exists() {
                schedule_delete_on_reboot(&install_dir);
            }
        }
    }

    Ok(())
}

/// Delete `path`; if it is in use, rename it aside and queue the renamed file
/// for deletion at reboot. Returns `true` if a reboot is needed.
fn remove_or_defer(path: &Path) -> bool {
    if !path.exists() || fs::remove_file(path).is_ok() {
        return false;
    }
    match rename_aside(path) {
        Ok(stale) => schedule_delete_on_reboot(&stale),
        Err(_) => schedule_delete_on_reboot(path),
    }
    true
}

/// Case-insensitive path equality (Windows file systems are case-insensitive;
/// `%ProgramFiles%` and `current_exe()` may differ only in case).
fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().to_lowercase();
    norm(a) == norm(b)
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
