//! `squareinput-imk --register`: register the .app this binary runs from as
//! an input source and enable it, so a first install needs no logging out.
//! When it was not enabled before, it also becomes the current input source.
//! The install script runs this after copying the app into place.
//!
//! Text Input Source Services (Carbon) has no Cocoa equivalent. Two catches:
//! a process caches the source list, so whether enabling took is read back in
//! a fresh child process (`--finish-register`); and the system rescans a new
//! bundle a few seconds after installing, which can undo the first attempt,
//! so success is confirmed twice and the whole thing retried for a while.

use std::ffi::c_void;
use std::path::Path;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use objc2_core_foundation::{CFArray, CFBoolean, CFDictionary, CFRetained, CFString, CFURL};
use objc2_foundation::NSBundle;

/// An opaque input source (TISInputSourceRef).
#[repr(C)]
struct TISInputSource {
    _private: [u8; 0],
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    /// Add an input method bundle to the system's input sources (again is harmless).
    fn TISRegisterInputSource(location: NonNull<CFURL>) -> i32;
    /// Input sources matching a property dictionary, enabled or not when
    /// `include_all_installed`. The caller owns the array.
    fn TISCreateInputSourceList(properties: *const c_void, include_all_installed: u8) -> *mut CFArray;
    fn TISEnableInputSource(source: *const TISInputSource) -> i32;
    fn TISSelectInputSource(source: *const TISInputSource) -> i32;
    /// A property of a source; the system owns the value.
    fn TISGetInputSourceProperty(source: *const TISInputSource, key: NonNull<CFString>) -> *const c_void;
    static kTISPropertyInputSourceID: NonNull<CFString>;
    static kTISPropertyInputSourceIsEnabled: NonNull<CFString>;
}

/// The child process's flag: `--finish-register <source id> <select: 0 or 1>`.
pub const FINISH_FLAG: &str = "--finish-register";
/// Child exit codes.
const ENABLED: i32 = 0;
const NOT_ENABLED: i32 = 1;
const SELECT_FAILED: i32 = 2;

/// Wait after enabling before reading back.
const SETTLE: Duration = Duration::from_millis(1500);
/// Wait before confirming again: the rescan of a new bundle comes 3-5 s after installing.
const CONFIRM: Duration = Duration::from_secs(3);
const RETRY: Duration = Duration::from_millis(500);
const TIMEOUT: Duration = Duration::from_secs(30);

/// Input sources with ID `id`, enabled or not.
fn sources(id: &str) -> Option<CFRetained<CFArray>> {
    // SAFETY: the key is a constant the framework exports.
    let key: &CFString = unsafe { kTISPropertyInputSourceID.as_ref() };
    let value = CFString::from_str(id);
    let filter = CFDictionary::<CFString, CFString>::from_slices(&[key], &[&value]);
    // SAFETY: a valid dictionary; the returned array follows the Create rule,
    // so from_raw takes ownership of it.
    unsafe {
        let raw = TISCreateInputSourceList(CFRetained::as_ptr(&filter).as_ptr().cast::<c_void>(), 1);
        NonNull::new(raw).map(|ptr| CFRetained::from_raw(ptr))
    }
}

/// The sources in `array`, valid while it lives.
fn each(array: &CFArray) -> Vec<*const TISInputSource> {
    // SAFETY: indices are in range and the elements are TISInputSourceRefs.
    (0..array.count()).map(|i| unsafe { array.value_at_index(i) }.cast::<TISInputSource>()).collect()
}

fn is_enabled(source: *const TISInputSource) -> bool {
    // SAFETY: `source` comes from a live array; the value is a CFBoolean the
    // system owns, only read here.
    unsafe {
        let value = TISGetInputSourceProperty(source, kTISPropertyInputSourceIsEnabled);
        value.cast::<CFBoolean>().as_ref().is_some_and(CFBoolean::value)
    }
}

/// Whether every source with ID `id` is enabled, as this process sees it.
fn all_enabled(id: &str) -> bool {
    sources(id).is_some_and(|s| {
        let list = each(&s);
        !list.is_empty() && list.into_iter().all(is_enabled)
    })
}

/// The input mode's source ID: the bundle identifier plus ".Hant", as in Info.plist.
fn mode_id(bundle: &NSBundle) -> Option<String> {
    bundle.bundleIdentifier().map(|id| format!("{id}.Hant"))
}

/// `--register`; returns the process exit code.
pub fn register() -> i32 {
    match try_register() {
        Ok(message) => {
            println!("{message}");
            0
        }
        Err(message) => {
            eprintln!("{message}");
            1
        }
    }
}

fn try_register() -> Result<String, String> {
    let bundle = NSBundle::mainBundle();
    let path = bundle.bundlePath().to_string();
    if !path.ends_with(".app") {
        return Err(format!("not run from an .app: {path}"));
    }
    let id = mode_id(&bundle).ok_or("no bundle identifier")?;
    let url = CFURL::from_file_path(Path::new(&path)).ok_or_else(|| format!("bad path: {path}"))?;
    let exe = std::env::current_exe().map_err(|e| format!("cannot find this executable: {e}"))?;
    // Fresh process, so this is current: switch to it only on a first install.
    let select = !all_enabled(&id);
    let started = Instant::now();
    loop {
        // SAFETY: a valid CFURL, only read.
        let status = unsafe { TISRegisterInputSource(CFRetained::as_ptr(&url)) };
        if status != 0 {
            return Err(format!("TISRegisterInputSource failed ({status})"));
        }
        if let Some(list) = sources(&id) {
            let list = each(&list);
            for &source in &list {
                // SAFETY: from a live array.
                let status = unsafe { TISEnableInputSource(source) };
                if status != 0 {
                    return Err(format!("TISEnableInputSource failed ({status})"));
                }
            }
            if !list.is_empty() {
                std::thread::sleep(SETTLE);
                if finish_in_child(&exe, &id, select)? != NOT_ENABLED {
                    std::thread::sleep(CONFIRM);
                    match finish_in_child(&exe, &id, select)? {
                        ENABLED if select => return Ok("registered, enabled and selected Square Input".into()),
                        ENABLED => return Ok("registered and enabled Square Input".into()),
                        SELECT_FAILED => return Ok("registered and enabled Square Input; pick it from the input menu".into()),
                        _ => {}
                    }
                }
            }
        }
        if started.elapsed() > TIMEOUT {
            return Err(format!("{id} did not become enabled within {} s", TIMEOUT.as_secs()));
        }
        std::thread::sleep(RETRY);
    }
}

fn finish_in_child(exe: &Path, id: &str, select: bool) -> Result<i32, String> {
    std::process::Command::new(exe)
        .args([FINISH_FLAG, id, if select { "1" } else { "0" }])
        .status()
        .map(|s| s.code().unwrap_or(NOT_ENABLED))
        .map_err(|e| format!("cannot start the read-back process: {e}"))
}

/// `--finish-register <id> <select>`, in a fresh process: whether every source
/// with ID `id` is enabled, selecting them if asked. Returns the exit code.
pub fn finish(id: Option<String>, select: Option<String>) -> i32 {
    let Some(id) = id else { return NOT_ENABLED };
    let Some(list) = sources(&id) else { return NOT_ENABLED };
    let list = each(&list);
    if list.is_empty() || !list.iter().all(|&s| is_enabled(s)) {
        return NOT_ENABLED;
    }
    if select.as_deref() == Some("1") {
        for &source in &list {
            // SAFETY: from a live array.
            if unsafe { TISSelectInputSource(source) } != 0 {
                return SELECT_FAILED;
            }
        }
    }
    ENABLED
}
