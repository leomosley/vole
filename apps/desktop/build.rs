use embed_manifest::manifest::ExecutionLevel;
use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    slint_build::compile("ui/app.slint").expect("Slint UI should compile");

    // Windows suppresses WM_HOTKEY delivery to lower-integrity processes while an
    // elevated window (e.g. an anti-cheat protected game) holds the foreground.
    // Requiring administrator rights runs VOLE at high integrity so hotkeys fire.
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let manifest =
            new_manifest("VOLE").requested_execution_level(ExecutionLevel::RequireAdministrator);
        embed_manifest(manifest).expect("Windows manifest should embed");
    }

    println!("cargo:rerun-if-changed=build.rs");
}
