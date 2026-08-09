#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    vole::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("VOLE runs on Windows only");
}
