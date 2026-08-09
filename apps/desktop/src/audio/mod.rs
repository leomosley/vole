use std::collections::HashMap;

use anyhow::Result;

use crate::config::{HotkeyBinding, Operation, Target};

pub mod fake;

#[cfg(windows)]
mod windows_backend;

#[cfg(windows)]
pub use windows_backend::WindowsBackend;

#[cfg(windows)]
pub type PlatformBackend = WindowsBackend;

#[cfg(not(windows))]
pub type PlatformBackend = fake::FakeBackend;

// Builds the audio backend for the current platform: real Core Audio on
// Windows, an in-memory fake everywhere else so the app stays runnable and
// testable on Linux.
pub fn platform_backend() -> Result<PlatformBackend> {
    #[cfg(windows)]
    {
        WindowsBackend::new()
    }
    #[cfg(not(windows))]
    {
        Ok(fake::FakeBackend::new())
    }
}

// A single audio session as seen by the engine. Executable is the process
// file name (e.g. `Discord.exe`) when the platform can resolve it.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionInfo {
    pub pid: u32,
    pub executable: Option<String>,
    pub level: f32,
    pub muted: bool,
}

// The primitive operations every platform must provide. All state changes are
// keyed by process id so the engine can stay platform independent.
pub trait AudioBackend {
    fn sessions(&self) -> Result<Vec<SessionInfo>>;
    fn set_level(&self, pid: u32, level: f32) -> Result<()>;
    fn set_mute(&self, pid: u32, muted: bool) -> Result<()>;
    fn foreground_pid(&self) -> Option<u32>;
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SessionState {
    level: f32,
    muted: bool,
}

// The result of firing a binding, used to tell the user when a hotkey matched
// no audio sessions instead of silently doing nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ApplyOutcome {
    pub affected: usize,
    pub restored: bool,
}

// Platform independent hotkey dispatch. Applies a binding's actions to the
// matching audio sessions and, for toggle bindings, restores the captured
// state on the next press.
pub struct AudioEngine<B> {
    backend: B,
    toggles: HashMap<String, HashMap<u32, SessionState>>,
}

impl<B: AudioBackend> AudioEngine<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            toggles: HashMap::new(),
        }
    }

    #[must_use]
    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn sessions(&self) -> Result<Vec<SessionInfo>> {
        self.backend.sessions()
    }

    pub fn applications(&self) -> Result<Vec<String>> {
        let mut names = self
            .backend
            .sessions()?
            .into_iter()
            .filter_map(|session| session.executable)
            .collect::<Vec<_>>();
        names.sort_unstable_by_key(|name| name.to_ascii_lowercase());
        names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        Ok(names)
    }

    pub fn apply(&mut self, binding: &HotkeyBinding) -> Result<ApplyOutcome> {
        let foreground = self.backend.foreground_pid();
        let sessions = self.backend.sessions()?;

        if binding.toggle
            && let Some(restored) = self.restore(binding, &sessions)?
        {
            return Ok(ApplyOutcome {
                affected: restored,
                restored: true,
            });
        }

        let mut affected = 0;
        for action in &binding.actions {
            for session in &sessions {
                if matches(session, &action.target, foreground) {
                    self.run(session, action.operation)?;
                    affected += 1;
                }
            }
        }

        Ok(ApplyOutcome {
            affected,
            restored: false,
        })
    }

    fn restore(
        &mut self,
        binding: &HotkeyBinding,
        sessions: &[SessionInfo],
    ) -> Result<Option<usize>> {
        if let Some(snapshot) = self.toggles.remove(&binding.id) {
            let mut restored = 0;
            for session in sessions {
                if let Some(state) = snapshot.get(&session.pid) {
                    self.backend.set_level(session.pid, state.level)?;
                    self.backend.set_mute(session.pid, state.muted)?;
                    restored += 1;
                }
            }
            return Ok(Some(restored));
        }

        let foreground = self.backend.foreground_pid();
        let mut snapshot = HashMap::new();
        for action in &binding.actions {
            for session in sessions {
                if matches(session, &action.target, foreground)
                    && !snapshot.contains_key(&session.pid)
                {
                    snapshot.insert(
                        session.pid,
                        SessionState {
                            level: session.level,
                            muted: session.muted,
                        },
                    );
                }
            }
        }
        self.toggles.insert(binding.id.clone(), snapshot);
        Ok(None)
    }

    fn run(&self, session: &SessionInfo, operation: Operation) -> Result<()> {
        match operation {
            Operation::Set { level } => self.backend.set_level(session.pid, level),
            Operation::Adjust { delta } => {
                self.backend.set_level(session.pid, session.level + delta)
            }
            Operation::Mute { muted } => self.backend.set_mute(session.pid, muted),
        }
    }
}

fn matches(session: &SessionInfo, target: &Target, foreground: Option<u32>) -> bool {
    match target {
        Target::Foreground => foreground == Some(session.pid),
        Target::Process { executable } => session
            .executable
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case(executable)),
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{FakeBackend, FakeSession};
    use super::*;
    use crate::config::Action;

    fn session(pid: u32, executable: &str, level: f32) -> FakeSession {
        FakeSession {
            pid,
            executable: executable.to_owned(),
            level,
            muted: false,
        }
    }

    fn binding(id: &str, toggle: bool, actions: Vec<Action>) -> HotkeyBinding {
        HotkeyBinding {
            id: id.to_owned(),
            name: id.to_owned(),
            shortcut: "Ctrl+Alt+D".to_owned(),
            enabled: true,
            toggle,
            custom_name: false,
            actions,
        }
    }

    fn process_action(executable: &str, operation: Operation) -> Action {
        Action {
            target: Target::Process {
                executable: executable.to_owned(),
            },
            operation,
        }
    }

    #[test]
    fn set_should_change_matching_process_level() {
        let backend = FakeBackend::with_sessions(vec![session(1, "Discord.exe", 1.0)]);
        let mut engine = AudioEngine::new(backend);

        let outcome = engine
            .apply(&binding(
                "one",
                false,
                vec![process_action("Discord.exe", Operation::Set { level: 0.2 })],
            ))
            .expect("apply should succeed");

        assert_eq!(outcome.affected, 1);
        assert_eq!(engine.backend().session(1).unwrap().level, 0.2);
    }

    #[test]
    fn apply_should_report_zero_when_nothing_matches() {
        let backend = FakeBackend::with_sessions(vec![session(1, "Discord.exe", 1.0)]);
        let mut engine = AudioEngine::new(backend);

        let outcome = engine
            .apply(&binding(
                "one",
                false,
                vec![process_action("nowhere.exe", Operation::Set { level: 0.2 })],
            ))
            .expect("apply should succeed");

        assert_eq!(outcome.affected, 0);
        assert_eq!(engine.backend().session(1).unwrap().level, 1.0);
    }

    #[test]
    fn process_match_should_ignore_case() {
        let backend = FakeBackend::with_sessions(vec![session(1, "Discord.exe", 1.0)]);
        let mut engine = AudioEngine::new(backend);

        engine
            .apply(&binding(
                "one",
                false,
                vec![process_action(
                    "discord.exe",
                    Operation::Mute { muted: true },
                )],
            ))
            .expect("apply should succeed");

        assert!(engine.backend().session(1).unwrap().muted);
    }

    #[test]
    fn foreground_action_should_only_touch_focused_process() {
        let backend = FakeBackend::with_sessions(vec![
            session(1, "Discord.exe", 1.0),
            session(2, "chrome.exe", 1.0),
        ]);
        backend.set_foreground(Some(2));
        let mut engine = AudioEngine::new(backend);

        engine
            .apply(&binding(
                "one",
                false,
                vec![Action {
                    target: Target::Foreground,
                    operation: Operation::Set { level: 0.1 },
                }],
            ))
            .expect("apply should succeed");

        assert_eq!(engine.backend().session(1).unwrap().level, 1.0);
        assert_eq!(engine.backend().session(2).unwrap().level, 0.1);
    }

    #[test]
    fn toggle_should_restore_previous_state_on_second_press() {
        let backend = FakeBackend::with_sessions(vec![session(1, "Discord.exe", 0.8)]);
        let mut engine = AudioEngine::new(backend);
        let hotkey = binding(
            "toggle",
            true,
            vec![process_action(
                "Discord.exe",
                Operation::Mute { muted: true },
            )],
        );

        engine.apply(&hotkey).expect("first press should mute");
        assert!(engine.backend().session(1).unwrap().muted);

        let outcome = engine.apply(&hotkey).expect("second press should restore");
        assert!(outcome.restored);
        let restored = engine.backend().session(1).unwrap();
        assert!(!restored.muted);
        assert_eq!(restored.level, 0.8);
    }

    #[test]
    fn adjust_should_clamp_within_range() {
        let backend = FakeBackend::with_sessions(vec![session(1, "Discord.exe", 0.9)]);
        let mut engine = AudioEngine::new(backend);

        engine
            .apply(&binding(
                "one",
                false,
                vec![process_action(
                    "Discord.exe",
                    Operation::Adjust { delta: 0.5 },
                )],
            ))
            .expect("apply should succeed");

        assert_eq!(engine.backend().session(1).unwrap().level, 1.0);
    }
}
