//! regsvr32 entry points: the COM class plus the TSF profile and keyboard
//! category. Needs an elevated process: TSF profiles live in HKLM.

use std::iter::once;

use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
use windows::Win32::UI::TextServices::*;
use windows::core::*;

use crate::{CLSID_TEXT_SERVICE, DESCRIPTION, GUID_PROFILE, LANGID};

/// TSF categories we register under.
///
/// - UIELEMENTENABLED: hosts that run their thread manager UI-less
///   (TF_TMAE_UIELEMENTENABLEDONLY) silently skip text services without it.
///   We keep drawing our own window there.
/// - IMMERSIVESUPPORT: text input through TextInputFramework (Windows
///   Terminal, Store apps) only cycles to text services declaring it; without
///   it Win+Space skips straight past us. In app containers the data under
///   %APPDATA% is unreadable, so there the service stays inert.
const CATEGORIES: [GUID; 3] =
    [GUID_TFCAT_TIP_KEYBOARD, GUID_TFCAT_TIPCAP_UIELEMENTENABLED, GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT];

fn clsid_key() -> String {
    format!("CLSID\\{{{CLSID_TEXT_SERVICE:?}}}")
}

fn module_path() -> Result<String> {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(Some(crate::module()), &mut buf) } as usize;
    if n == 0 || n >= buf.len() {
        return Err(Error::from_thread());
    }
    Ok(String::from_utf16_lossy(&buf[..n]))
}

struct RegKey(HKEY);

impl Drop for RegKey {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

impl RegKey {
    fn create(parent: HKEY, path: &str) -> Result<RegKey> {
        let mut key = HKEY::default();
        unsafe {
            RegCreateKeyExW(parent, &HSTRING::from(path), None, None, REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None)
                .ok()?;
        }
        Ok(RegKey(key))
    }

    fn set(&self, name: Option<&str>, value: &str) -> Result<()> {
        // REG_SZ data: UTF-16LE with a terminating NUL.
        let bytes: Vec<u8> = value.encode_utf16().chain(once(0)).flat_map(u16::to_le_bytes).collect();
        let name = name.map(HSTRING::from);
        let name = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
        unsafe { RegSetValueExW(self.0, name, None, REG_SZ, Some(&bytes)).ok() }
    }
}

fn with_com<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let result = f();
    if init.is_ok() {
        unsafe { CoUninitialize() };
    }
    result
}

pub fn register() -> Result<()> {
    let path = module_path()?;
    let class = RegKey::create(HKEY_CLASSES_ROOT, &clsid_key())?;
    class.set(None, DESCRIPTION)?;
    let server = RegKey::create(class.0, "InprocServer32")?;
    server.set(None, &path)?;
    server.set(Some("ThreadingModel"), "Apartment")?;

    with_com(|| unsafe {
        let profiles: ITfInputProcessorProfileMgr =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
        let description: Vec<u16> = DESCRIPTION.encode_utf16().collect();
        let icon: Vec<u16> = path.encode_utf16().collect();
        profiles.RegisterProfile(
            &CLSID_TEXT_SERVICE, LANGID, &GUID_PROFILE, &description, &icon, 0, HKL::default(), 0, true, 0,
        )?;
        let categories: ITfCategoryMgr = CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        for category in &CATEGORIES {
            categories.RegisterCategory(&CLSID_TEXT_SERVICE, category, &CLSID_TEXT_SERVICE)?;
        }
        Ok(())
    })
}

pub fn unregister() -> Result<()> {
    with_com(|| unsafe {
        let categories: ITfCategoryMgr = CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        for category in &CATEGORIES {
            let _ = categories.UnregisterCategory(&CLSID_TEXT_SERVICE, category, &CLSID_TEXT_SERVICE);
        }
        let profiles: ITfInputProcessorProfileMgr =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
        profiles.UnregisterProfile(&CLSID_TEXT_SERVICE, LANGID, &GUID_PROFILE, 0)
    })?;
    unsafe { RegDeleteTreeW(HKEY_CLASSES_ROOT, &HSTRING::from(clsid_key())).ok() }
}
