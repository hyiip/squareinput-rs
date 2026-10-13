//! The only module allowed `unsafe`. The rest of the crate is compiled with
//! `#![deny(unsafe_code)]`, so this file is the whole of what needs auditing.
//!
//! It holds four things:
//! - the COM object declarations, because `#[implement]` expands to unsafe
//!   code (their methods live in safe modules: `service.rs`);
//! - the input mode button, whole, as TSF hands its methods raw pointers;
//! - the DLL exports, where COM hands us raw pointers;
//! - thin safe wrappers for every Windows API the crate calls.
//!
//! Safe wrappers never take a raw pointer from their caller: arguments are
//! owned Rust values, COM interface references, or window handles (which
//! Windows validates itself). Raw pointers appear only in COM entry points
//! (the class factory, `DllGetClassObject` and the input mode button), where
//! COM supplies them and they are checked for null first.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::iter::once;
use std::mem::ManuallyDrop;
use std::sync::Once;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW,
    GetModuleHandleExW,
};
use windows::Win32::System::Ole::{CONNECT_E_ADVISELIMIT, CONNECT_E_CANNOTCONNECT, CONNECT_E_NOCONNECTION};
use windows::Win32::System::Registry::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, HKL, VIRTUAL_KEY};
use windows::Win32::UI::TextServices::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

use crate::{CLSID_TEXT_SERVICE, DESCRIPTION, data, guard};

// ---- COM objects --------------------------------------------------------

/// The text service. Its interface methods are in `service.rs`.
#[implement(ITfTextInputProcessorEx, ITfKeyEventSink, ITfThreadMgrEventSink, ITfThreadFocusSink)]
pub struct TextService {
    pub(crate) state: RefCell<crate::service::State>,
}

/// Inserts one committed string; see `service.rs`.
#[implement(ITfEditSession)]
pub struct CommitSession {
    pub(crate) context: ITfContext,
    pub(crate) text: Vec<u16>,
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Ref<IUnknown>, iid: *const GUID, object: *mut *mut c_void) -> Result<()> {
        guard(Err(E_UNEXPECTED.into()), || {
            if object.is_null() || iid.is_null() {
                return Err(E_POINTER.into());
            }
            // SAFETY: COM passes a valid out-pointer; it is non-null.
            unsafe { *object = std::ptr::null_mut() };
            if outer.ok().is_ok() {
                return Err(CLASS_E_NOAGGREGATION.into());
            }
            let service: IUnknown = TextService::new().into();
            // SAFETY: `iid` and `object` come from COM and are non-null.
            unsafe { service.query(iid, object) }.ok()
        })
    }

    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

// ---- Input mode button ------------------------------------------------------

/// Resource ids of the icons that build.rs embeds: the grid, and 英 for English.
const ICON_ID: usize = 1;
const ENGLISH_ICON_ID: usize = 2;

/// The only cookie [`ModeButton`] hands out: it takes a single sink.
const SINK_COOKIE: u32 = 1;

/// The input mode button: TSF shows its icon on the taskbar while we are the
/// active text service, the grid in Chinese and 英 in English. It only shows
/// the icon; clicking it does nothing.
#[implement(ITfLangBarItemButton, ITfSource)]
pub struct ModeButton {
    sink: RefCell<Option<ITfLangBarItemSink>>,
    english: Cell<bool>,
}

impl ModeButton {
    pub fn new(english: bool) -> ComObject<ModeButton> {
        ComObject::new(ModeButton { sink: RefCell::default(), english: Cell::new(english) })
    }

    /// Show the icon for `english`, telling TSF if it changed.
    pub fn show_english(&self, english: bool) {
        if self.english.replace(english) == english {
            return;
        }
        let sink = self.sink.try_borrow().ok().and_then(|s| s.clone());
        if let Some(sink) = sink {
            let _ = unsafe { sink.OnUpdate(TF_LBI_ICON) };
        }
    }
}

pub fn langbar_item(button: &ComObject<ModeButton>) -> ITfLangBarItem {
    button.to_interface::<ITfLangBarItemButton>().into()
}

impl ITfLangBarItem_Impl for ModeButton_Impl {
    fn GetInfo(&self, info: *mut TF_LANGBARITEMINFO) -> Result<()> {
        guard(Err(E_UNEXPECTED.into()), || {
            if info.is_null() {
                return Err(E_INVALIDARG.into());
            }
            let mut description = [0u16; 32];
            for (d, s) in description.iter_mut().zip(DESCRIPTION.encode_utf16().take(31)) {
                *d = s;
            }
            // SAFETY: TSF passes a valid out-pointer; it is non-null.
            unsafe {
                *info = TF_LANGBARITEMINFO {
                    clsidService: CLSID_TEXT_SERVICE,
                    guidItem: GUID_LBI_INPUTMODE,
                    dwStyle: TF_LBI_STYLE_BTN_BUTTON | TF_LBI_STYLE_SHOWNINTRAY,
                    ulSort: 0,
                    szDescription: description,
                };
            }
            Ok(())
        })
    }

    fn GetStatus(&self) -> Result<u32> {
        Ok(0)
    }

    fn Show(&self, _show: BOOL) -> Result<()> {
        Ok(())
    }

    fn GetTooltipString(&self) -> Result<BSTR> {
        Ok(BSTR::from(DESCRIPTION))
    }
}

impl ITfLangBarItemButton_Impl for ModeButton_Impl {
    fn OnClick(&self, _click: TfLBIClick, _point: &POINT, _area: *const RECT) -> Result<()> {
        Ok(())
    }

    fn InitMenu(&self, _menu: Ref<ITfMenu>) -> Result<()> {
        Ok(())
    }

    fn OnMenuSelect(&self, _id: u32) -> Result<()> {
        Ok(())
    }

    /// TSF destroys the icon when it is done with it, so each call loads a new one.
    fn GetIcon(&self) -> Result<HICON> {
        guard(Err(E_UNEXPECTED.into()), || {
            let size = unsafe { GetSystemMetrics(SM_CXSMICON) };
            let id = if self.english.get() { ENGLISH_ICON_ID } else { ICON_ID };
            let name = PCWSTR(std::ptr::without_provenance(id));
            let icon = unsafe { LoadImageW(Some(module().into()), name, IMAGE_ICON, size, size, LR_DEFAULTCOLOR) }?;
            Ok(HICON(icon.0))
        })
    }

    fn GetText(&self) -> Result<BSTR> {
        Ok(BSTR::from(DESCRIPTION))
    }
}

impl ITfSource_Impl for ModeButton_Impl {
    fn AdviseSink(&self, iid: *const GUID, sink: Ref<IUnknown>) -> Result<u32> {
        guard(Err(E_UNEXPECTED.into()), || {
            if iid.is_null() {
                return Err(E_INVALIDARG.into());
            }
            // SAFETY: TSF passes a valid IID pointer; it is non-null.
            if unsafe { *iid } != ITfLangBarItemSink::IID {
                return Err(CONNECT_E_CANNOTCONNECT.into());
            }
            let sink: ITfLangBarItemSink = sink.ok()?.cast()?;
            let mut slot = self.sink.try_borrow_mut().map_err(|_| Error::from(E_UNEXPECTED))?;
            if slot.is_some() {
                return Err(CONNECT_E_ADVISELIMIT.into());
            }
            *slot = Some(sink);
            Ok(SINK_COOKIE)
        })
    }

    fn UnadviseSink(&self, cookie: u32) -> Result<()> {
        guard(Err(E_UNEXPECTED.into()), || {
            let mut slot = self.sink.try_borrow_mut().map_err(|_| Error::from(E_UNEXPECTED))?;
            if cookie != SINK_COOKIE || slot.take().is_none() {
                return Err(CONNECT_E_NOCONNECTION.into());
            }
            Ok(())
        })
    }
}

// ---- DLL exports ----------------------------------------------------------

/// # Safety
/// Called by COM, which passes valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(clsid: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    guard(E_UNEXPECTED, || {
        if out.is_null() || clsid.is_null() || iid.is_null() {
            return E_POINTER;
        }
        // SAFETY: all three pointers come from COM and are non-null.
        let clsid = unsafe {
            *out = std::ptr::null_mut();
            *clsid
        };
        data::debug(|| format!("DllGetClassObject {clsid:?}"));
        if clsid != CLSID_TEXT_SERVICE {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory.into();
        // SAFETY: as above.
        unsafe { factory.query(iid, out) }
    })
}

/// Never unload: windows, sinks and caches stay valid for the life of the process.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

#[unsafe(no_mangle)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    guard(E_UNEXPECTED, || crate::register::register().map_or_else(|e| e.code(), |()| S_OK))
}

#[unsafe(no_mangle)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    guard(E_UNEXPECTED, || crate::register::unregister().map_or_else(|e| e.code(), |()| S_OK))
}

// ---- This DLL -------------------------------------------------------------

fn module() -> HMODULE {
    static ANCHOR: u8 = 0;
    let mut module = HMODULE::default();
    // SAFETY: with FROM_ADDRESS the "name" is only an address inside this DLL.
    let _ = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(&raw const ANCHOR as *const u16),
            &mut module,
        )
    };
    module
}

/// Full path of this DLL.
pub fn module_path() -> Result<String> {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(Some(module()), &mut buf) } as usize;
    if n == 0 || n >= buf.len() {
        return Err(Error::from_thread());
    }
    Ok(String::from_utf16_lossy(&buf[..n]))
}

// ---- Registry and COM registration (regsvr32 only) -------------------------

/// An open registry key, closed on drop.
pub struct RegKey(HKEY);

impl Drop for RegKey {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

fn create_key(parent: HKEY, path: &str) -> Result<RegKey> {
    let mut key = HKEY::default();
    unsafe {
        RegCreateKeyExW(parent, &HSTRING::from(path), None, None, REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None)
            .ok()?;
    }
    Ok(RegKey(key))
}

impl RegKey {
    /// Create (or open) `path` under HKEY_CLASSES_ROOT.
    pub fn classes(path: &str) -> Result<RegKey> {
        create_key(HKEY_CLASSES_ROOT, path)
    }

    pub fn subkey(&self, path: &str) -> Result<RegKey> {
        create_key(self.0, path)
    }

    /// Set a string value; `None` is the key's default value.
    pub fn set(&self, name: Option<&str>, value: &str) -> Result<()> {
        // REG_SZ data: UTF-16LE with a terminating NUL.
        let bytes: Vec<u8> = value.encode_utf16().chain(once(0)).flat_map(u16::to_le_bytes).collect();
        let name = name.map(HSTRING::from).unwrap_or_default();
        unsafe { RegSetValueExW(self.0, &name, None, REG_SZ, Some(&bytes)).ok() }
    }
}

/// Delete `path` under HKEY_CLASSES_ROOT with everything below it.
pub fn delete_classes_tree(path: &str) -> Result<()> {
    unsafe { RegDeleteTreeW(HKEY_CLASSES_ROOT, &HSTRING::from(path)).ok() }
}

/// Run `f` with COM initialised on this thread.
pub fn with_com<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let result = f();
    if init.is_ok() {
        unsafe { CoUninitialize() };
    }
    result
}

fn create<T: Interface>(clsid: &GUID) -> Result<T> {
    unsafe { CoCreateInstance(clsid, None, CLSCTX_INPROC_SERVER) }
}

pub fn register_profile(clsid: &GUID, langid: u16, profile: &GUID, description: &str, icon_file: &str) -> Result<()> {
    let profiles: ITfInputProcessorProfileMgr = create(&CLSID_TF_InputProcessorProfiles)?;
    let description: Vec<u16> = description.encode_utf16().collect();
    let icon_file: Vec<u16> = icon_file.encode_utf16().collect();
    unsafe { profiles.RegisterProfile(clsid, langid, profile, &description, &icon_file, 0, HKL::default(), 0, true, 0) }
}

pub fn unregister_profile(clsid: &GUID, langid: u16, profile: &GUID) -> Result<()> {
    let profiles: ITfInputProcessorProfileMgr = create(&CLSID_TF_InputProcessorProfiles)?;
    unsafe { profiles.UnregisterProfile(clsid, langid, profile, 0) }
}

pub fn register_category(clsid: &GUID, category: &GUID) -> Result<()> {
    let categories: ITfCategoryMgr = create(&CLSID_TF_CategoryMgr)?;
    unsafe { categories.RegisterCategory(clsid, category, clsid) }
}

pub fn unregister_category(clsid: &GUID, category: &GUID) -> Result<()> {
    let categories: ITfCategoryMgr = create(&CLSID_TF_CategoryMgr)?;
    unsafe { categories.UnregisterCategory(clsid, category, clsid) }
}

// ---- TSF ------------------------------------------------------------------

/// Whether `vk` is held down, as of the message being processed.
pub fn key_held(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(i32::from(vk.0)) < 0 }
}

/// Advise `sink` on `source` under its own interface IID; returns the cookie.
pub fn advise_sink<T: Interface>(source: &ITfSource, sink: &T) -> Result<u32> {
    let unknown: IUnknown = sink.cast()?;
    unsafe { source.AdviseSink(&T::IID, &unknown) }
}

pub fn unadvise_sink(source: &ITfSource, cookie: u32) {
    let _ = unsafe { source.UnadviseSink(cookie) };
}

/// Receive the thread's keystrokes as its foreground text service.
pub fn advise_key_sink(keystrokes: &ITfKeystrokeMgr, client_id: u32, sink: &ITfKeyEventSink) -> Result<()> {
    unsafe { keystrokes.AdviseKeyEventSink(client_id, sink, true) }
}

pub fn unadvise_key_sink(keystrokes: &ITfKeystrokeMgr, client_id: u32) {
    let _ = unsafe { keystrokes.UnadviseKeyEventSink(client_id) };
}

/// Show `item` (on the taskbar) while this thread's text service is active.
pub fn add_langbar_item(thread_mgr: &ITfThreadMgr, item: &ITfLangBarItem) -> Result<()> {
    let items: ITfLangBarItemMgr = thread_mgr.cast()?;
    unsafe { items.AddItem(item) }
}

pub fn remove_langbar_item(thread_mgr: &ITfThreadMgr, item: &ITfLangBarItem) {
    if let Ok(items) = thread_mgr.cast::<ITfLangBarItemMgr>() {
        let _ = unsafe { items.RemoveItem(item) };
    }
}

/// Whether a document of this thread has TSF focus.
pub fn has_focus_document(thread_mgr: &ITfThreadMgr) -> bool {
    unsafe { thread_mgr.GetFocus() }.is_ok()
}

/// Ask for a read-write edit session, synchronous when possible.
pub fn request_edit_session(context: &ITfContext, client_id: u32, session: &ITfEditSession) -> Result<HRESULT> {
    unsafe { context.RequestEditSession(client_id, session, TF_ES_ASYNCDONTCARE | TF_ES_READWRITE) }
}

/// Insert `text` at the selection, as a composition that ends at once
/// (applications that only understand IME compositions need that), and put
/// the caret after it. Call from inside an edit session with cookie `ec`.
pub fn insert_text(context: &ITfContext, ec: u32, text: &[u16]) -> Result<()> {
    unsafe {
        let insert: ITfInsertAtSelection = context.cast()?;
        let range = insert.InsertTextAtSelection(ec, TF_IAS_QUERYONLY, &[])?;
        let composition = context
            .cast::<ITfContextComposition>()
            .and_then(|c| c.StartComposition(ec, &range, None::<&ITfCompositionSink>));
        let target = match &composition {
            Ok(c) => c.GetRange()?,
            Err(_) => range,
        };
        target.SetText(ec, 0, text)?;
        let caret = target.Clone()?;
        caret.Collapse(ec, TF_ANCHOR_END)?;
        let selection = TF_SELECTION {
            range: ManuallyDrop::new(Some(caret)),
            style: TF_SELECTIONSTYLE { ase: TF_AE_NONE, fInterimChar: FALSE },
        };
        let set = context.SetSelection(ec, std::slice::from_ref(&selection));
        drop(ManuallyDrop::into_inner(selection.range));
        if let Ok(c) = composition {
            c.EndComposition(ec)?;
        }
        set
    }
}

// ---- Windows and GDI --------------------------------------------------------

const CLASS: PCWSTR = w!("squareinput-candidates");

/// A window procedure written in safe code.
pub type WindowProc = extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

/// Register the candidate window class (once per process).
pub fn register_window_class(proc: WindowProc) {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT),
            hInstance: module().into(),
            lpszClassName: CLASS,
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
            ..Default::default()
        };
        unsafe { RegisterClassW(&wc) };
    });
}

/// A hidden topmost popup of the candidate class that never takes focus.
pub fn create_popup(title: &str, width: i32, height: i32) -> Result<HWND> {
    unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            CLASS,
            &HSTRING::from(title),
            WS_POPUP,
            0,
            0,
            width,
            height,
            None,
            None,
            Some(module().into()),
            None,
        )
    }
}

pub fn default_window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

pub fn is_visible(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd) }.as_bool()
}

/// Scale from 96-dpi layout units to the window's pixels.
pub fn dpi_scale(hwnd: HWND) -> f32 {
    unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0
}

/// Work area of the monitor showing the foreground window.
pub fn foreground_work_area() -> Option<RECT> {
    unsafe {
        let monitor = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        GetMonitorInfoW(monitor, &mut info).as_bool().then_some(info.rcWork)
    }
}

/// Move, size and show `hwnd` topmost without activating it.
pub fn place_topmost(hwnd: HWND, x: i32, y: i32, width: i32, height: i32) {
    let _ = unsafe { SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW) };
}

pub fn invalidate(hwnd: HWND) {
    let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
}

pub fn validate(hwnd: HWND) {
    let _ = unsafe { ValidateRect(Some(hwnd), None) };
}

pub fn hide(hwnd: HWND) {
    let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
}

pub fn destroy(hwnd: HWND) {
    let _ = unsafe { DestroyWindow(hwnd) };
}

/// Handle WM_PAINT: `draw` paints into an off-screen buffer the size of the
/// client area, which is then copied in one go so the window never flickers.
/// If `draw` returns false nothing is copied.
pub fn paint_buffered(hwnd: HWND, draw: impl FnOnce(&Canvas) -> bool) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let (w, h) = (rc.right, rc.bottom);
        let mem = CreateCompatibleDC(Some(hdc));
        let bitmap = CreateCompatibleBitmap(hdc, w, h);
        let old = SelectObject(mem, bitmap.into());
        SetBkMode(mem, TRANSPARENT);
        if draw(&Canvas { hdc: mem, scale: dpi_scale(hwnd) }) {
            let _ = BitBlt(hdc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
        }
        SelectObject(mem, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem);
        let _ = EndPaint(hwnd, &ps);
    }
}

/// A device context to draw on, only lent out by [`paint_buffered`].
pub struct Canvas {
    hdc: HDC,
    /// Pixels per 96-dpi layout unit.
    pub scale: f32,
}

impl Canvas {
    pub fn fill(&self, r: &RECT, color: COLORREF) {
        unsafe {
            let brush = CreateSolidBrush(color);
            FillRect(self.hdc, r, brush);
            let _ = DeleteObject(brush.into());
        }
    }

    pub fn frame(&self, r: &RECT, color: COLORREF) {
        unsafe {
            let brush = CreateSolidBrush(color);
            FrameRect(self.hdc, r, brush);
            let _ = DeleteObject(brush.into());
        }
    }

    /// Draw `text` in `r` with a font `height` pixels high.
    pub fn text(&self, text: &str, r: &RECT, height: i32, face: &str, color: COLORREF, format: DRAW_TEXT_FORMAT) {
        if text.is_empty() {
            return;
        }
        let face = HSTRING::from(face);
        let mut buf: Vec<u16> = text.encode_utf16().collect();
        let mut r = *r;
        unsafe {
            let font = CreateFontW(
                -height, 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY, (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32, &face,
            );
            let old = SelectObject(self.hdc, font.into());
            SetTextColor(self.hdc, color);
            DrawTextW(self.hdc, &mut buf, &mut r, format | DT_NOPREFIX);
            SelectObject(self.hdc, old);
            let _ = DeleteObject(font.into());
        }
    }
}
