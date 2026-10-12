pub mod common;
pub mod ime;
pub mod registration;

use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{
    BOOL, CLASS_E_CLASSNOTAVAILABLE, E_FAIL, E_INVALIDARG, E_POINTER, HMODULE, MAX_PATH, S_FALSE,
    S_OK, TRUE,
};
use windows::Win32::System::LibraryLoader::{DisableThreadLibraryCalls, GetModuleFileNameW};
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows_core::{HRESULT, IUnknown, Interface, GUID};

use crate::common::IME_CLSID;
use crate::ime::{ClassFactory, GLOBAL_OBJECT_COUNT};

static DLL_INSTANCE: AtomicIsize = AtomicIsize::new(0);

#[unsafe(no_mangle)]
pub extern "system" fn DllMain(
    hinst_dll: HMODULE,
    fdw_reason: u32,
    _lpv_reserved: *const c_void,
) -> BOOL {
    if fdw_reason == DLL_PROCESS_ATTACH {
        DLL_INSTANCE.store(hinst_dll.0 as isize, Ordering::SeqCst);
        unsafe {
            let _ = DisableThreadLibraryCalls(hinst_dll);
        }
    }
    TRUE
}

#[unsafe(no_mangle)]
pub extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() {
        return E_POINTER;
    }
    unsafe {
        *ppv = std::ptr::null_mut();
    }
    if rclsid.is_null() || riid.is_null() {
        return E_INVALIDARG;
    }
    let clsid = unsafe { *rclsid };
    if clsid != IME_CLSID {
        return CLASS_E_CLASSNOTAVAILABLE;
    }

    let factory: IUnknown = ClassFactory::new().into();
    unsafe { factory.query(riid, ppv) }
}

#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if GLOBAL_OBJECT_COUNT.load(Ordering::SeqCst) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    let raw_handle = DLL_INSTANCE.load(Ordering::SeqCst);
    let hmodule = HMODULE(raw_handle as *mut c_void);

    let mut buf = [0u16; (MAX_PATH as usize) * 4];
    let len = unsafe { GetModuleFileNameW(hmodule, &mut buf) } as usize;
    if len == 0 {
        return E_FAIL;
    }

    let dll_path = String::from_utf16_lossy(&buf[..len]);
    match registration::register_all_ime(&dll_path) {
        Ok(()) => S_OK,
        Err(e) => e.code(),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    match registration::unregister_all_ime() {
        Ok(()) => S_OK,
        Err(e) => e.code(),
    }
}
