#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("--enable-autostart") => vole::set_autostart(true),
        Some("--disable-autostart") => vole::set_autostart(false),
        _ => vole::run(),
    }
}
