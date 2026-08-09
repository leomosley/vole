pub mod audio;
pub mod autostart;
pub mod catalog;
pub mod config;

mod app;

slint::include_modules!();

pub use app::run;

use anyhow::Result;

use crate::config::ConfigStore;

// Toggles logon autostart and records the choice in the config so the setting
// survives restarts and stays in step with the scheduled task. The installer
// invokes this through the CLI while elevated.
pub fn set_autostart(enabled: bool) -> Result<()> {
    autostart::set(enabled)?;

    let store = ConfigStore::new()?;
    let mut config = store.load()?;
    if config.launch_on_startup != enabled {
        config.launch_on_startup = enabled;
        store.save(&config)?;
    }
    Ok(())
}
