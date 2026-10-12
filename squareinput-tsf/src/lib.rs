//! Square Input: a 字形 input method for Windows, as a TSF text service.
//!
//! The DLL runs inside every application: every entry point catches panics; it
//! never touches the network, the working directory or other processes; and
//! without its data it simply lets every key through.

mod data;
mod labels;
mod register;
mod service;
mod window;

use std::ffi::c_void;
use std::panic::AssertUnwindSafe;

use windows::Win32::Foundation::*;
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleHandleExW,
};
use windows::core::*;

pub const CLSID_TEXT_SERVICE: GUID = GUID::from_u128(0x89B0ACE9_CFE0_4081_A7C3_7FBFB6082420);
pub const GUID_PROFILE: GUID = GUID::from_u128(0x8D43BCFA_667D_4A00_9383_968A0E98609E);
/// Registered under zh-TW; Windows lists it with the 中文(香港) keyboards.
pub const LANGID: u16 = 0x0404;
pub const DESCRIPTION: &str = "Square Input";

/// Run `f`, turning a panic into `fallback`: nothing may unwind into the host.
pub(crate) fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    std::panic::catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        data::log("caught a panic");
        fallback
    })
}

/// This DLL's module handle.
pub(crate) fn module() -> HMODULE {
    static ANCHOR: u8 = 0;
    let mut module = HMODULE::default();
    unsafe {
        let _ = GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(&raw const ANCHOR as *const u16),
            &mut module,
        );
    }
    module
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Ref<IUnknown>, iid: *const GUID, object: *mut *mut c_void) -> Result<()> {
        guard(Err(E_UNEXPECTED.into()), || unsafe {
            if object.is_null() {
                return Err(E_POINTER.into());
            }
            *object = std::ptr::null_mut();
            if outer.ok().is_ok() {
                return Err(CLASS_E_NOAGGREGATION.into());
            }
            let service: IUnknown = service::TextService::new().into();
            service.query(iid, object).ok()
        })
    }

    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

/// # Safety
/// Called by COM with valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(clsid: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    guard(E_UNEXPECTED, || unsafe {
        if out.is_null() || clsid.is_null() {
            return E_POINTER;
        }
        data::debug(|| format!("DllGetClassObject {:?}", *clsid));
        *out = std::ptr::null_mut();
        if *clsid != CLSID_TEXT_SERVICE {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory.into();
        factory.query(iid, out)
    })
}

/// Never unload: windows, sinks and caches stay valid for the life of the process.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

#[unsafe(no_mangle)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    guard(E_UNEXPECTED, || register::register().map_or_else(|e| e.code(), |()| S_OK))
}

#[unsafe(no_mangle)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    guard(E_UNEXPECTED, || register::unregister().map_or_else(|e| e.code(), |()| S_OK))
}
