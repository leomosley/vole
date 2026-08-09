// End-to-end verification of the hotkey dispatch pipeline on any platform.
// Loads a config exactly as the GUI would save it, then fires each hotkey
// through the same engine the real global hotkeys use and asserts the audio
// state changed. This runs against the in-memory fake backend so it works on
// Linux CI without Windows.

use std::time::{SystemTime, UNIX_EPOCH};

use vole::audio::AudioEngine;
use vole::audio::fake::{FakeBackend, FakeSession};
use vole::config::{ConfigStore, HotkeyBinding};

fn playing(pid: u32, executable: &str, level: f32) -> FakeSession {
    FakeSession {
        pid,
        executable: executable.to_owned(),
        level,
        muted: false,
    }
}

fn temp_config_path() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should follow Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("vole-e2e-{nonce}.json"))
}

fn find(config: &[HotkeyBinding], id: &str) -> HotkeyBinding {
    config
        .iter()
        .find(|binding| binding.id == id)
        .cloned()
        .unwrap_or_else(|| panic!("binding `{id}` should exist"))
}

#[test]
fn config_should_drive_audio_end_to_end() {
    let path = temp_config_path();
    let store = ConfigStore::at(path.clone());
    let saved = r#"{
        "version": 2,
        "launch_on_startup": false,
        "hotkeys": [
            {
                "id": "duck-discord",
                "name": "Duck Discord",
                "shortcut": "Ctrl+Alt+D",
                "enabled": true,
                "toggle": false,
                "actions": [
                    { "target": { "type": "process", "executable": "Discord.exe" }, "operation": { "type": "set", "level": 0.2 } }
                ]
            },
            {
                "id": "mute-spotify",
                "name": "Mute Spotify",
                "shortcut": "Ctrl+Alt+S",
                "enabled": true,
                "toggle": true,
                "actions": [
                    { "target": { "type": "process", "executable": "spotify.exe" }, "operation": { "type": "mute", "muted": true } }
                ]
            }
        ]
    }"#;
    std::fs::write(&path, saved).expect("config should write");

    let config = store.load().expect("config should load");
    let _ = std::fs::remove_file(&path);

    let backend = FakeBackend::with_sessions(vec![
        playing(101, "Discord.exe", 1.0),
        playing(103, "Spotify.exe", 0.7),
    ]);
    let mut engine = AudioEngine::new(backend);

    // Firing the duck hotkey should set Discord to 20% and leave Spotify alone.
    let duck = engine
        .apply(&find(&config.hotkeys, "duck-discord"))
        .expect("duck should apply");
    assert_eq!(duck.affected, 1);
    assert_eq!(engine.backend().session(101).unwrap().level, 0.2);
    assert_eq!(engine.backend().session(103).unwrap().level, 0.7);

    // The toggle mute should mute Spotify, then restore it on a second press.
    let mute = find(&config.hotkeys, "mute-spotify");
    engine.apply(&mute).expect("mute should apply");
    assert!(engine.backend().session(103).unwrap().muted);

    engine.apply(&mute).expect("mute toggle should restore");
    let restored = engine.backend().session(103).unwrap();
    assert!(!restored.muted);
    assert_eq!(restored.level, 0.7);
}

#[test]
fn disabled_shortcut_still_dispatches_when_fired_directly() {
    // The engine itself does not gate on `enabled`; that is the OS registration
    // layer's job. Firing a binding always runs its actions, which is what the
    // in-app test trigger relies on.
    let backend = FakeBackend::with_sessions(vec![playing(1, "chrome.exe", 1.0)]);
    let mut engine = AudioEngine::new(backend);

    let binding = HotkeyBinding {
        id: "x".to_owned(),
        name: "x".to_owned(),
        shortcut: "Ctrl+Alt+X".to_owned(),
        enabled: false,
        toggle: false,
        actions: vec![vole::config::Action {
            target: vole::config::Target::Process {
                executable: "chrome.exe".to_owned(),
            },
            operation: vole::config::Operation::Mute { muted: true },
        }],
    };

    engine.apply(&binding).expect("apply should succeed");
    assert!(engine.backend().session(1).unwrap().muted);
}

#[test]
fn firing_a_hotkey_with_no_matching_session_reports_zero() {
    // This is the "nothing happens on press" case: the target app is not
    // playing audio, so no session matches. The engine now reports this so the
    // UI can tell the user instead of appearing broken.
    let backend = FakeBackend::with_sessions(vec![playing(1, "chrome.exe", 1.0)]);
    let mut engine = AudioEngine::new(backend);

    let binding = HotkeyBinding {
        id: "y".to_owned(),
        name: "y".to_owned(),
        shortcut: "Ctrl+Alt+Y".to_owned(),
        enabled: true,
        toggle: false,
        actions: vec![vole::config::Action {
            target: vole::config::Target::Process {
                executable: "discord.exe".to_owned(),
            },
            operation: vole::config::Operation::Mute { muted: true },
        }],
    };

    let outcome = engine.apply(&binding).expect("apply should succeed");
    assert_eq!(outcome.affected, 0);
}
