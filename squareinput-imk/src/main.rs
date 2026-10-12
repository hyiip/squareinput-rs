//! Square Input for macOS: an InputMethodKit input method. The engine,
//! window layout and data come from the `squareinput` core; this crate only
//! feeds it numpad keys, inserts what it commits and draws its window.
//!
//! It cannot run with `cargo run`: `scripts/install-macos.sh` bundles it into
//! an .app in `~/Library/Input Methods`.

#[cfg(target_os = "macos")]
mod controller;
#[cfg(target_os = "macos")]
mod register;
#[cfg(target_os = "macos")]
mod window;

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("squareinput-imk runs on macOS only");
}

#[cfg(target_os = "macos")]
fn main() {
    use objc2::{AnyThread, ClassType, MainThreadMarker};
    use objc2_app_kit::NSApplication;
    use objc2_foundation::{NSBundle, NSString};
    use objc2_input_method_kit::IMKServer;
    use squareinput::store;

    // Run by the install script rather than by the system: register the input source and quit.
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--register") => std::process::exit(register::register()),
        Some(register::FINISH_FLAG) => std::process::exit(register::finish(args.next(), args.next())),
        _ => {}
    }

    let Some(mtm) = MainThreadMarker::new() else {
        store::log("not started on the main thread");
        std::process::exit(1);
    };
    std::panic::set_hook(Box::new(|info| store::log(&format!("panic: {info}"))));

    // IMKServer looks the controller class up by name (InputMethodServerControllerClass)
    // as soon as it starts, but define_class! registers it only on first use: without
    // this, IMK silently falls back to the base class and every key passes through.
    let _ = controller::Controller::class();

    let Some(identifier) = NSBundle::mainBundle().bundleIdentifier() else {
        store::log("no bundle identifier: run it from the installed .app");
        std::process::exit(1);
    };
    let connection = NSString::from_str(&format!("{identifier}_Connection"));
    // SAFETY: called once on the main thread with valid strings; the server
    // lives as long as the process.
    let server = unsafe { IMKServer::initWithName_bundleIdentifier(IMKServer::alloc(), Some(&connection), Some(&identifier)) };
    let Some(_server) = server else {
        store::log("cannot start the IMK server: check InputMethodConnectionName in Info.plist");
        std::process::exit(1);
    };
    store::log("started");
    NSApplication::sharedApplication(mtm).run();
}
