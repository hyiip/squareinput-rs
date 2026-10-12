//! Square Input: a shape-based Chinese input method for Windows, as a TSF text service.
//!
//! The DLL runs inside every application: every entry point catches panics; it
//! never touches the network, the working directory or other processes; and
//! without its data it simply lets every key through.
//!
//! `unsafe` is denied everywhere except `sys`, which holds the COM objects,
//! the DLL exports and thin safe wrappers for the Windows APIs used.
#![cfg(windows)]
#![deny(unsafe_code)]

mod register;
mod service;
#[allow(unsafe_code)]
mod sys;
mod window;

use std::panic::AssertUnwindSafe;

pub(crate) use squareinput::store as data;
use windows::core::GUID;

pub use sys::{DllCanUnloadNow, DllGetClassObject, DllRegisterServer, DllUnregisterServer};

pub const CLSID_TEXT_SERVICE: GUID = GUID::from_u128(0x89B0ACE9_CFE0_4081_A7C3_7FBFB6082420);
pub const GUID_PROFILE: GUID = GUID::from_u128(0x8D43BCFA_667D_4A00_9383_968A0E98609E);
/// Registered under zh-TW; Windows lists it with the Chinese (Hong Kong) keyboards.
pub const LANGID: u16 = 0x0404;
pub const DESCRIPTION: &str = "Square Input";

/// Run `f`, turning a panic into `fallback`: nothing may unwind into the host.
pub(crate) fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    std::panic::catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        data::log("caught a panic");
        fallback
    })
}
