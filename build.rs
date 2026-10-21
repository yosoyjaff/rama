//! Embeds the application icon in the Windows executable: the window, the
//! taskbar and Explorer all read it from the binary's resources. Linux takes
//! its icon from the `.desktop` entry and macOS from the `.app` bundle, so on
//! those targets there is nothing to do here.

fn main() {
    println!("cargo:rerun-if-changed=assets/app-icon/app.rama.rc");
    println!("cargo:rerun-if-changed=assets/app-icon/app.rama.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/app-icon/app.rama.rc", embed_resource::NONE)
            .manifest_required()
            .expect("Windows app icon resource compilation failed");
    }
}
