//! regsvr32 entry points: the COM class plus the TSF profile and categories.
//! Needs an elevated process: TSF profiles live in HKLM.

use windows::Win32::UI::TextServices::{
    GUID_TFCAT_TIP_KEYBOARD, GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT, GUID_TFCAT_TIPCAP_UIELEMENTENABLED,
};
use windows::core::{GUID, Result};

use crate::sys::{self, RegKey};
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

pub fn register() -> Result<()> {
    let path = sys::module_path()?;
    let class = RegKey::classes(&clsid_key())?;
    class.set(None, DESCRIPTION)?;
    let server = class.subkey("InprocServer32")?;
    server.set(None, &path)?;
    server.set(Some("ThreadingModel"), "Apartment")?;

    sys::with_com(|| {
        sys::register_profile(&CLSID_TEXT_SERVICE, LANGID, &GUID_PROFILE, DESCRIPTION, &path)?;
        for category in &CATEGORIES {
            sys::register_category(&CLSID_TEXT_SERVICE, category)?;
        }
        Ok(())
    })
}

pub fn unregister() -> Result<()> {
    sys::with_com(|| {
        for category in &CATEGORIES {
            let _ = sys::unregister_category(&CLSID_TEXT_SERVICE, category);
        }
        sys::unregister_profile(&CLSID_TEXT_SERVICE, LANGID, &GUID_PROFILE)
    })?;
    sys::delete_classes_tree(&clsid_key())
}
