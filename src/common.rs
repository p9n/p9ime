use windows_core::GUID;

/// Unique CLSID for the P9IME TSF Text Input Processor (COM Server).
/// {A3D7F1C2-5B84-4E96-8F0D-2C6B9E7A1D43}
pub const IME_CLSID: GUID = GUID::from_u128(0xa3d7f1c2_5b84_4e96_8f0d_2c6b9e7a1d43);
pub const IME_CLSID_STR: &str = "{A3D7F1C2-5B84-4E96-8F0D-2C6B9E7A1D43}";

/// Unique Profile GUID for the P9IME language profile.
/// {E58B2C94-7A13-4F6D-B1E7-9D4A3C8F5B62}
pub const IME_PROFILE_GUID: GUID = GUID::from_u128(0xe58b2c94_7a13_4f6d_b1e7_9d4a3c8f5b62);

/// Display name shown in Windows 10/11 Language Bar / Input Switcher and Installed Apps.
pub const IME_NAME: &str = "P9IME";
pub const IME_PUBLISHER: &str = "P9IME";
pub const IME_VERSION: &str = "0.1.0";

/// Language ID the IME profile is registered under. Windows Settings lists the IME
/// under this language's "Keyboards" (the language must be added to the user's
/// language list first).
///   0x0404 = Chinese (Traditional, Taiwan)
///   0x0804 = Chinese (Simplified, PRC)
///   0x0409 = English (United States)
pub const IME_LANGID: u16 = 0x0404;

/// Subdirectory under `%ProgramFiles%` where the IME components are installed.
pub const INSTALL_DIR_NAME: &str = "P9IME";
pub const DLL_FILENAME: &str = "p9ime.dll";
pub const UNINSTALLER_FILENAME: &str = "uninstaller.exe";

/// Registry path for Windows "Installed Apps" / "Programs and Features" uninstaller.
pub const UNINSTALL_REG_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\P9IME";

/// Convert a Rust UTF-8 string into a null-terminated UTF-16 buffer for Win32 APIs.
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
