//! P9IME setup: a single self-contained binary that installs or uninstalls.
//!
//! * `p9ime-installer.exe` → install (the DLL is embedded, nothing to unzip)
//! * `p9ime-installer.exe --uninstall` → uninstall
//! * `uninstaller.exe` → uninstall (the installer copies itself under this
//!   name into the install dir, so the registered UninstallString and a
//!   double-click both work)
//!
//! `--quiet` (or `/S`, `/quiet`) suppresses the message boxes in either mode.

mod install;
mod ui;
mod uninstall;

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use p9ime::common::{IME_NAME, INSTALL_DIR_NAME, UNINSTALLER_FILENAME, UNINSTALL_FLAG};

/// `%ProgramFiles%\P9IME`
pub(crate) fn install_dir() -> PathBuf {
    let program_files =
        env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string());
    PathBuf::from(program_files).join(INSTALL_DIR_NAME)
}

fn running_as_uninstaller() -> bool {
    env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_os_string()))
        .is_some_and(|n| n.eq_ignore_ascii_case(UNINSTALLER_FILENAME))
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let quiet = args
        .iter()
        .any(|a| a == "--quiet" || a == "/quiet" || a == "/S");
    let uninstall = args
        .iter()
        .any(|a| a == UNINSTALL_FLAG || a == "/uninstall")
        || running_as_uninstaller();

    let (label, result) = if uninstall {
        ("Uninstaller", uninstall::run(quiet))
    } else {
        ("Installer", install::run(quiet))
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            ui::show_message(&format!("{} {} Error", IME_NAME, label), &err, true, quiet);
            ExitCode::FAILURE
        }
    }
}
