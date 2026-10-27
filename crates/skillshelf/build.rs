//! Embeds the app icon in the Windows executable, where Explorer, the taskbar and the Start menu
//! take it from. gpui embeds the application manifest itself.

const ICON: &str = "../../assets/brand/skillshelf.ico";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=skillshelf.rc");
    println!("cargo:rerun-if-changed={ICON}");
    // `mise run icon` makes it, and building that example runs this script too.
    if !std::path::Path::new(ICON).exists() {
        println!("cargo:warning={ICON} is missing; run `mise run icon`");
        return;
    }
    embed_resource::compile("skillshelf.rc", embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
