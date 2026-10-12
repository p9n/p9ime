use p9ime::common::to_wide;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK,
};
use windows_core::PCWSTR;

/// Print to the console and, unless `quiet`, show a modal message box.
pub fn show_message(title: &str, message: &str, is_error: bool, quiet: bool) {
    if is_error {
        eprintln!("{}: {}", title, message);
    } else {
        println!("{}: {}", title, message);
    }
    if quiet {
        return;
    }
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
