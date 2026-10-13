//! Embeds the icons (resources/*.ico, drawn by resources/make-icon.ps1) into
//! the DLL: the grid as its first icon, which the TSF profile and the
//! taskbar's input mode button show, and 英 for the button in English.

fn main() {
    #[cfg(windows)]
    embed_icons();
}

#[cfg(windows)]
fn embed_icons() {
    use std::path::PathBuf;

    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
    let mut script = String::new();
    // Resource ids as in sys.rs.
    for (id, name) in [(1, "squareinput.ico"), (2, "english.ico")] {
        println!("cargo:rerun-if-changed=resources/{name}");
        // Name the icon by an absolute path, so the resource compiler's search
        // rules for relative ones never matter.
        let escaped = resources.join(name).display().to_string().replace('\\', "\\\\");
        script += &format!("{id} ICON \"{escaped}\"\n");
    }
    let rc = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("squareinput.rc");
    std::fs::write(&rc, script).expect("cannot write squareinput.rc");
    embed_resource::compile_for_cdylib(&rc, embed_resource::NONE).manifest_required().expect("cannot embed the icons");
}
