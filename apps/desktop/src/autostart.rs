use anyhow::Result;

// Autostart runs VOLE through a logon-triggered scheduled task with the highest
// privileges, rather than an `HKCU\...\Run` entry. Because VOLE now requires
// administrator rights, a Run entry would raise a UAC prompt at every logon; a
// task registered with `/RL HIGHEST` launches elevated silently instead.

pub fn set(enabled: bool) -> Result<()> {
    if enabled {
        imp::enable()
    } else {
        imp::disable()
    }
}

#[must_use]
pub fn is_enabled() -> bool {
    imp::is_enabled()
}

#[cfg(windows)]
mod imp {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    use anyhow::{Context, Result, bail};

    const TASK_NAME: &str = "VOLE Autostart";
    // Keep schtasks from flashing a console window in the windowed subsystem.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    fn schtasks() -> Command {
        let mut command = Command::new("schtasks.exe");
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }

    pub fn enable() -> Result<()> {
        let exe = std::env::current_exe().context("failed to locate the VOLE executable")?;
        let program = exe
            .to_str()
            .context("the VOLE executable path is not valid UTF-8")?;
        // schtasks needs the /TR value wrapped in quotes so spaces in the
        // install path stay part of a single command.
        let target = format!("\"{program}\"");

        let status = schtasks()
            .args([
                "/Create", "/TN", TASK_NAME, "/TR", &target, "/SC", "ONLOGON", "/RL", "HIGHEST",
                "/F",
            ])
            .status()
            .context("failed to run schtasks to create the autostart task")?;

        if !status.success() {
            bail!("schtasks failed to create the autostart task ({status})");
        }
        Ok(())
    }

    pub fn disable() -> Result<()> {
        if !is_enabled() {
            return Ok(());
        }

        let status = schtasks()
            .args(["/Delete", "/TN", TASK_NAME, "/F"])
            .status()
            .context("failed to run schtasks to delete the autostart task")?;

        if !status.success() {
            bail!("schtasks failed to delete the autostart task ({status})");
        }
        Ok(())
    }

    pub fn is_enabled() -> bool {
        schtasks()
            .args(["/Query", "/TN", TASK_NAME])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

#[cfg(not(windows))]
mod imp {
    use anyhow::Result;

    pub fn enable() -> Result<()> {
        Ok(())
    }

    pub fn disable() -> Result<()> {
        Ok(())
    }

    #[must_use]
    pub fn is_enabled() -> bool {
        false
    }
}
