use embed_manifest::manifest::ExecutionLevel;
use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    slint_build::compile("ui/app.slint").expect("Slint UI should compile");

    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        // Windows suppresses WM_HOTKEY delivery to lower-integrity processes while an
        // elevated window (e.g. an anti-cheat protected game) holds the foreground.
        // Requiring administrator rights runs VOLE at high integrity so hotkeys fire.
        let manifest =
            new_manifest("VOLE").requested_execution_level(ExecutionLevel::RequireAdministrator);
        embed_manifest(manifest).expect("Windows manifest should embed");

        // Embed the mascot as the executable's own icon so Explorer, the Start-menu
        // shortcut, and the taskbar show it. This needs a resource compiler; the
        // shipped release builds with the MSVC toolchain (rc.exe), so scope it there
        // and leave other Windows targets (e.g. gnu) without the embedded icon.
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            let _ = embed_resource::compile("app.rc", embed_resource::NONE);
        }
    }

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=../../assets/vole.ico");
}
