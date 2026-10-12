use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, TRUE};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE,
    KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
use windows::Win32::UI::TextServices::{
    CLSID_TF_CategoryMgr, CLSID_TF_InputProcessorProfiles, ITfCategoryMgr,
    ITfInputProcessorProfileMgr, ITfInputProcessorProfiles, GUID_TFCAT_TIPCAP_COMLESS,
    GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT, GUID_TFCAT_TIPCAP_INPUTMODECOMPARTMENT,
    GUID_TFCAT_TIPCAP_SECUREMODE, GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
    GUID_TFCAT_TIPCAP_UIELEMENTENABLED, GUID_TFCAT_TIP_KEYBOARD,
};
use windows_core::{Interface, Result, GUID, PCWSTR};

use crate::common::{
    to_wide, IME_CLSID, IME_CLSID_STR, IME_LANGID, IME_NAME, IME_PROFILE_GUID, IME_PUBLISHER,
    IME_VERSION, UNINSTALL_FLAG, UNINSTALL_REG_KEY,
};

/// Categories required for a Windows 10/11 TSF IME (including UWP/Immersive and System Tray).
const TSF_CATEGORIES: &[GUID] = &[
    GUID_TFCAT_TIP_KEYBOARD,
    GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,
    GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
    GUID_TFCAT_TIPCAP_SECUREMODE,
    GUID_TFCAT_TIPCAP_UIELEMENTENABLED,
    GUID_TFCAT_TIPCAP_INPUTMODECOMPARTMENT,
    GUID_TFCAT_TIPCAP_COMLESS,
];

unsafe fn set_reg_sz(hkey: HKEY, value_name: Option<&str>, data: &str) -> Result<()> {
    let name_wide = value_name.map(to_wide);
    let name_ptr = match &name_wide {
        Some(v) => PCWSTR(v.as_ptr()),
        None => PCWSTR::null(),
    };
    let data_wide = to_wide(data);
    let byte_slice =
        std::slice::from_raw_parts(data_wide.as_ptr() as *const u8, data_wide.len() * 2);
    RegSetValueExW(hkey, name_ptr, 0, REG_SZ, Some(byte_slice)).ok()
}

unsafe fn set_reg_dword(hkey: HKEY, value_name: &str, data: u32) -> Result<()> {
    let name_wide = to_wide(value_name);
    let bytes = data.to_le_bytes();
    RegSetValueExW(hkey, PCWSTR(name_wide.as_ptr()), 0, REG_DWORD, Some(&bytes)).ok()
}

/// Register the COM InprocServer32 entry under `HKLM\SOFTWARE\Classes\CLSID\{...}`.
pub fn register_com_server(dll_path: &str) -> Result<()> {
    unsafe {
        let clsid_key_path = format!(r"SOFTWARE\Classes\CLSID\{}", IME_CLSID_STR);
        let clsid_key_wide = to_wide(&clsid_key_path);
        let mut hkey_clsid = HKEY::default();
        RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(clsid_key_wide.as_ptr()),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey_clsid,
            None,
        )
        .ok()?;
        let res_default = set_reg_sz(hkey_clsid, None, IME_NAME);
        let _ = RegCloseKey(hkey_clsid);
        res_default?;

        let inproc_key_path = format!(r"{}\InprocServer32", clsid_key_path);
        let inproc_key_wide = to_wide(&inproc_key_path);
        let mut hkey_inproc = HKEY::default();
        RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(inproc_key_wide.as_ptr()),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey_inproc,
            None,
        )
        .ok()?;
        let res_path = set_reg_sz(hkey_inproc, None, dll_path);
        let res_model = set_reg_sz(hkey_inproc, Some("ThreadingModel"), "Apartment");
        let _ = RegCloseKey(hkey_inproc);
        res_path?;
        res_model?;
        Ok(())
    }
}

/// Remove the COM InprocServer32 entry under `HKLM\SOFTWARE\Classes\CLSID\{...}`.
pub fn unregister_com_server() -> Result<()> {
    unsafe {
        let clsid_key_path = format!(r"SOFTWARE\Classes\CLSID\{}", IME_CLSID_STR);
        let clsid_key_wide = to_wide(&clsid_key_path);
        let status = RegDeleteTreeW(HKEY_LOCAL_MACHINE, PCWSTR(clsid_key_wide.as_ptr()));
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            status.ok()
        }
    }
}

/// Register the TSF Input Processor Profile and Windows 10/11 TSF Categories.
pub fn register_tsf(dll_path: &str) -> Result<()> {
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let should_uninit = hr.is_ok();

        let result = (|| -> Result<()> {
            let profiles: ITfInputProcessorProfiles =
                CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
            profiles.Register(&IME_CLSID)?;

            let profile_mgr: ITfInputProcessorProfileMgr = profiles.cast()?;
            let desc_wide = to_wide(IME_NAME);
            let icon_file_wide = to_wide(dll_path);

            // Exclude trailing null when passing slice to RegisterProfile
            let desc_slice = &desc_wide[..desc_wide.len().saturating_sub(1)];
            let icon_slice = &icon_file_wide[..icon_file_wide.len().saturating_sub(1)];

            profile_mgr.RegisterProfile(
                &IME_CLSID,
                IME_LANGID,
                &IME_PROFILE_GUID,
                desc_slice,
                icon_slice,
                0,
                HKL::default(),
                0,
                TRUE,
                0,
            )?;

            let _ = profiles.EnableLanguageProfile(&IME_CLSID, IME_LANGID, &IME_PROFILE_GUID, TRUE);
            let _ = profiles.EnableLanguageProfileByDefault(
                &IME_CLSID,
                IME_LANGID,
                &IME_PROFILE_GUID,
                TRUE,
            );

            let cat_mgr: ITfCategoryMgr =
                CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
            for category in TSF_CATEGORIES {
                cat_mgr.RegisterCategory(&IME_CLSID, category, &IME_CLSID)?;
            }

            Ok(())
        })();

        if should_uninit {
            CoUninitialize();
        }
        result
    }
}

/// Unregister the TSF Input Processor Profile and Categories.
pub fn unregister_tsf() -> Result<()> {
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let should_uninit = hr.is_ok();

        if let Ok(cat_mgr) =
            CoCreateInstance::<_, ITfCategoryMgr>(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)
        {
            for category in TSF_CATEGORIES {
                let _ = cat_mgr.UnregisterCategory(&IME_CLSID, category, &IME_CLSID);
            }
        }

        if let Ok(profiles) = CoCreateInstance::<_, ITfInputProcessorProfiles>(
            &CLSID_TF_InputProcessorProfiles,
            None,
            CLSCTX_INPROC_SERVER,
        ) {
            if let Ok(profile_mgr) = profiles.cast::<ITfInputProcessorProfileMgr>() {
                let _ = profile_mgr.UnregisterProfile(&IME_CLSID, IME_LANGID, &IME_PROFILE_GUID, 0);
            }
            let _ = profiles.Unregister(&IME_CLSID);
        }

        if should_uninit {
            CoUninitialize();
        }
        Ok(())
    }
}

/// Register the uninstaller in Windows "Installed Apps" / "Programs and Features".
pub fn register_uninstaller(
    install_dir: &str,
    uninstaller_path: &str,
    dll_path: &str,
) -> Result<()> {
    unsafe {
        let key_wide = to_wide(UNINSTALL_REG_KEY);
        let mut hkey = HKEY::default();
        RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key_wide.as_ptr()),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        )
        .ok()?;

        let uninstall_cmd = format!("\"{}\" {}", uninstaller_path, UNINSTALL_FLAG);
        let quiet_uninstall_cmd = format!("\"{}\" {} --quiet", uninstaller_path, UNINSTALL_FLAG);
        let display_icon = format!("{},0", dll_path);

        let res = (|| -> Result<()> {
            set_reg_sz(hkey, Some("DisplayName"), IME_NAME)?;
            set_reg_sz(hkey, Some("UninstallString"), &uninstall_cmd)?;
            set_reg_sz(hkey, Some("QuietUninstallString"), &quiet_uninstall_cmd)?;
            set_reg_sz(hkey, Some("InstallLocation"), install_dir)?;
            set_reg_sz(hkey, Some("DisplayIcon"), &display_icon)?;
            set_reg_sz(hkey, Some("Publisher"), IME_PUBLISHER)?;
            set_reg_sz(hkey, Some("DisplayVersion"), IME_VERSION)?;
            set_reg_dword(hkey, "NoModify", 1)?;
            set_reg_dword(hkey, "NoRepair", 1)?;
            Ok(())
        })();

        let _ = RegCloseKey(hkey);
        res
    }
}

/// Remove the uninstaller entry from Windows "Installed Apps" / "Programs and Features".
pub fn unregister_uninstaller() -> Result<()> {
    unsafe {
        let key_wide = to_wide(UNINSTALL_REG_KEY);
        let status = RegDeleteTreeW(HKEY_LOCAL_MACHINE, PCWSTR(key_wide.as_ptr()));
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            status.ok()
        }
    }
}

/// Perform full IME registration (COM server + TSF profile/categories).
pub fn register_all_ime(dll_path: &str) -> Result<()> {
    register_com_server(dll_path)?;
    register_tsf(dll_path)?;
    Ok(())
}

/// Perform full IME unregistration (TSF profile/categories + COM server).
pub fn unregister_all_ime() -> Result<()> {
    let _ = unregister_tsf();
    unregister_com_server()?;
    Ok(())
}
