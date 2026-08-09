use std::cell::RefCell;

use anyhow::Result;

use super::{AudioBackend, SessionInfo};

// A simulated audio session used off Windows and in tests. Holds the same
// state the real Core Audio sessions expose so the engine behaves identically.
#[derive(Clone, Debug, PartialEq)]
pub struct FakeSession {
    pub pid: u32,
    pub executable: String,
    pub level: f32,
    pub muted: bool,
}

impl FakeSession {
    fn playing(pid: u32, executable: &str) -> Self {
        Self {
            pid,
            executable: executable.to_owned(),
            level: 1.0,
            muted: false,
        }
    }
}

// In-memory audio backend. Lets VOLE run and be exercised end to end without
// Windows, so hotkey dispatch can be verified on any machine.
pub struct FakeBackend {
    sessions: RefCell<Vec<FakeSession>>,
    foreground: RefCell<Option<u32>>,
}

impl FakeBackend {
    // A handful of common apps so the Linux GUI has something to route audio to.
    #[must_use]
    pub fn new() -> Self {
        Self::with_sessions(vec![
            FakeSession::playing(101, "Discord.exe"),
            FakeSession::playing(102, "chrome.exe"),
            FakeSession::playing(103, "Spotify.exe"),
            FakeSession::playing(104, "vlc.exe"),
        ])
    }

    #[must_use]
    pub fn with_sessions(sessions: Vec<FakeSession>) -> Self {
        let foreground = sessions.first().map(|session| session.pid);
        Self {
            sessions: RefCell::new(sessions),
            foreground: RefCell::new(foreground),
        }
    }

    pub fn set_foreground(&self, pid: Option<u32>) {
        *self.foreground.borrow_mut() = pid;
    }

    #[must_use]
    pub fn session(&self, pid: u32) -> Option<FakeSession> {
        self.sessions
            .borrow()
            .iter()
            .find(|session| session.pid == pid)
            .cloned()
    }

    #[must_use]
    pub fn snapshot(&self) -> Vec<FakeSession> {
        self.sessions.borrow().clone()
    }
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBackend for FakeBackend {
    fn sessions(&self) -> Result<Vec<SessionInfo>> {
        Ok(self
            .sessions
            .borrow()
            .iter()
            .map(|session| SessionInfo {
                pid: session.pid,
                executable: Some(session.executable.clone()),
                level: session.level,
                muted: session.muted,
            })
            .collect())
    }

    fn set_level(&self, pid: u32, level: f32) -> Result<()> {
        if let Some(session) = self
            .sessions
            .borrow_mut()
            .iter_mut()
            .find(|session| session.pid == pid)
        {
            session.level = level.clamp(0.0, 1.0);
        }
        Ok(())
    }

    fn set_mute(&self, pid: u32, muted: bool) -> Result<()> {
        if let Some(session) = self
            .sessions
            .borrow_mut()
            .iter_mut()
            .find(|session| session.pid == pid)
        {
            session.muted = muted;
        }
        Ok(())
    }

    fn foreground_pid(&self) -> Option<u32> {
        *self.foreground.borrow()
    }
}
