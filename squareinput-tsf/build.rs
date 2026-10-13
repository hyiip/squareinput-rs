//! Embeds the icon (resources/squareinput.ico, drawn by resources/make-icon.ps1)
//! into the DLL as its first icon, which the TSF profile and the taskbar's
//! input mode button show.

fn main() {
    #[cfg(windows)]
    embed_icon();
}

#[cfg(windows)]
fn embed_icon() {
    use std::path::PathBuf;

    println!("cargo:rerun-if-changed=resources/squareinput.ico");
    let ico = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join("squareinput.ico");
    // Name the icon by an absolute path, so the resource compiler's search
    // rules for relative ones never matter.
    let rc = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("squareinput.rc");
    let escaped = ico.display().to_string().replace('\\', "\\\\");
    std::fs::write(&rc, format!("1 ICON \"{escaped}\"\n")).expect("cannot write squareinput.rc");
    embed_resource::compile_for_cdylib(&rc, embed_resource::NONE).manifest_required().expect("cannot embed the icon");
}
