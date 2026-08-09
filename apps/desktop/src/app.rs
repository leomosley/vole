use std::cell::RefCell;

use anyhow::{Context, Result};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::audio::{AudioEngine, PlatformBackend, SessionInfo};
use crate::catalog::{self, AppEntry};
use crate::config::{Action, Config, ConfigStore, HotkeyBinding, Operation, Target};
use crate::{ActionRow, AppWindow, ApplicationRow, HotkeyRow};

#[cfg(windows)]
use global_hotkey::GlobalHotKeyManager;
#[cfg(windows)]
use global_hotkey::hotkey::HotKey;
#[cfg(windows)]
use std::collections::HashMap;
#[cfg(windows)]
use std::str::FromStr;

thread_local! {
    static RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) };
}

pub fn run() -> Result<()> {
    let ui = AppWindow::new().context("failed to create VOLE window")?;
    let store = ConfigStore::new()?;
    let config = store.load()?;
    let mut runtime = Runtime::new(store, config)?;
    runtime.register_hotkeys()?;
    runtime.refresh_applications()?;
    RUNTIME.with(|slot| slot.replace(Some(runtime)));

    bind_ui(&ui);
    refresh_ui(&ui)?;

    // Hotkey and tray events are drained from their channels by a timer running
    // on the Slint event loop, rather than through each crate's push handler.
    // Those handlers are stored in a `OnceCell` that locks to the first value it
    // sees, so a single early event can silently prevent the handler from ever
    // being installed. Polling the channel from the UI thread avoids that race
    // and keeps all dispatch on one thread.
    #[cfg(windows)]
    let (tray, open_id, quit_id) = create_tray(&ui)?;
    #[cfg(windows)]
    let _event_pump = spawn_event_pump(&ui, open_id, quit_id);

    ui.window()
        .on_close_requested(|| slint::CloseRequestResponse::HideWindow);

    ui.show().context("failed to show VOLE window")?;
    let result = slint::run_event_loop_until_quit().context("VOLE event loop failed");

    #[cfg(windows)]
    drop(tray);

    RUNTIME.with(|slot| slot.replace(None));
    result
}

struct Runtime {
    store: ConfigStore,
    config: Config,
    engine: AudioEngine<PlatformBackend>,
    applications: Vec<AppEntry>,
    application_search: String,
    #[cfg(windows)]
    hotkeys: GlobalHotKeyManager,
    #[cfg(windows)]
    bindings: HashMap<u32, usize>,
}

impl Runtime {
    fn new(store: ConfigStore, config: Config) -> Result<Self> {
        Ok(Self {
            store,
            config,
            engine: AudioEngine::new(crate::audio::platform_backend()?),
            applications: Vec::new(),
            application_search: String::new(),
            #[cfg(windows)]
            hotkeys: GlobalHotKeyManager::new().context("failed to initialize global hotkeys")?,
            #[cfg(windows)]
            bindings: HashMap::new(),
        })
    }

    fn refresh_applications(&mut self) -> Result<()> {
        let playing = self.engine.applications().unwrap_or_default();
        self.applications = catalog::build(&playing);
        Ok(())
    }

    fn save_config(&mut self, config: Config) -> Result<()> {
        config.validate().context("invalid configuration")?;

        self.unregister_hotkeys()?;
        let previous = std::mem::replace(&mut self.config, config);
        if let Err(error) = self.register_hotkeys() {
            self.config = previous;
            self.register_hotkeys()
                .context("failed to restore previous hotkeys")?;
            return Err(error);
        }

        if let Err(error) = self.store.save(&self.config) {
            self.unregister_hotkeys()?;
            self.config = previous;
            self.register_hotkeys()
                .context("failed to restore previous hotkeys")?;
            return Err(error.into());
        }

        Ok(())
    }

    fn mutate_config(&mut self, mutation: impl FnOnce(&mut Config)) -> Result<()> {
        let mut config = self.config.clone();
        mutation(&mut config);
        self.save_config(config)
    }

    // Runs a binding's actions through the audio engine. This is the single
    // dispatch path shared by real global hotkeys and the in-app test trigger.
    // Returns a human message describing what happened so silent no-ops (a
    // hotkey that matched no audio session) become visible instead of looking
    // like the app is broken.
    fn fire(&mut self, index: usize) -> Result<String> {
        let Some(binding) = self.config.hotkeys.get(index).cloned() else {
            return Ok("No hotkey to fire.".to_owned());
        };
        let outcome = self.engine.apply(&binding)?;
        Ok(describe_outcome(&binding, outcome))
    }

    #[cfg(windows)]
    fn register_hotkeys(&mut self) -> Result<()> {
        let mut registered = Vec::new();
        for (index, binding) in self.config.hotkeys.iter().enumerate() {
            if !binding.enabled {
                continue;
            }
            let hotkey = HotKey::from_str(&binding.shortcut)
                .with_context(|| format!("invalid shortcut `{}`", binding.shortcut))?;
            if let Err(error) = self.hotkeys.register(hotkey) {
                if !registered.is_empty() {
                    let _ = self.hotkeys.unregister_all(&registered);
                }
                self.bindings.clear();
                return Err(error)
                    .with_context(|| format!("shortcut `{}` is unavailable", binding.shortcut));
            }
            registered.push(hotkey);
            self.bindings.insert(hotkey.id(), index);
        }
        Ok(())
    }

    #[cfg(windows)]
    fn unregister_hotkeys(&mut self) -> Result<()> {
        let hotkeys = self
            .config
            .hotkeys
            .iter()
            .filter(|binding| binding.enabled)
            .map(|binding| {
                HotKey::from_str(&binding.shortcut)
                    .with_context(|| format!("invalid shortcut `{}`", binding.shortcut))
            })
            .collect::<Result<Vec<_>>>()?;

        if !hotkeys.is_empty() {
            self.hotkeys
                .unregister_all(&hotkeys)
                .context("failed to unregister hotkeys")?;
        }
        self.bindings.clear();
        Ok(())
    }

    #[cfg(not(windows))]
    fn register_hotkeys(&mut self) -> Result<()> {
        Ok(())
    }

    #[cfg(not(windows))]
    fn unregister_hotkeys(&mut self) -> Result<()> {
        Ok(())
    }

    #[cfg(windows)]
    fn handle_hotkey(&mut self, id: u32) -> Result<String> {
        let Some(&index) = self.bindings.get(&id) else {
            return Ok(String::new());
        };
        self.fire(index)
    }
}

fn with_runtime<T>(operation: impl FnOnce(&mut Runtime) -> Result<T>) -> Result<T> {
    RUNTIME.with(|slot| {
        let mut runtime = slot.borrow_mut();
        let runtime = runtime.as_mut().context("VOLE runtime is unavailable")?;
        operation(runtime)
    })
}

fn bind_ui(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.on_search_applications(move |query| {
        let _ = with_runtime(|runtime| {
            runtime.application_search = query.to_string();
            Ok(())
        });
        update_ui(&weak, "Application list filtered.");
    });

    let weak = ui.as_weak();
    ui.on_refresh_applications(move || {
        run_ui_action(&weak, "Applications refreshed.", |runtime| {
            runtime.refresh_applications()
        });
    });

    let weak = ui.as_weak();
    ui.on_select_hotkey(move |index| {
        if let Some(ui) = weak.upgrade() {
            select_hotkey(&ui, index);
        }
    });

    let weak = ui.as_weak();
    ui.on_add_hotkey(move || {
        run_ui_action(&weak, "Hotkey created.", |runtime| {
            let sequence = runtime.config.hotkeys.len() + 1;
            let shortcut = next_shortcut(&runtime.config);
            runtime.mutate_config(|config| {
                config.hotkeys.push(HotkeyBinding {
                    id: format!("hotkey-{sequence}"),
                    name: format!("Hotkey {sequence}"),
                    shortcut,
                    enabled: true,
                    toggle: false,
                    actions: vec![Action {
                        target: Target::Foreground,
                        operation: Operation::Set { level: 0.5 },
                    }],
                });
            })
        });
    });

    let weak = ui.as_weak();
    ui.on_remove_hotkey(move |index| {
        if let Some(ui) = weak.upgrade() {
            ui.set_selected_hotkey(-1);
            ui.set_selected_action(-1);
        }
        run_ui_action(&weak, "Hotkey removed.", |runtime| {
            runtime.mutate_config(|config| {
                if index >= 0 && (index as usize) < config.hotkeys.len() {
                    config.hotkeys.remove(index as usize);
                }
            })
        });
    });

    let weak = ui.as_weak();
    ui.on_test_fire(move |index| {
        let message = with_runtime(|runtime| runtime.fire(index.max(0) as usize));
        let Some(ui) = weak.upgrade() else {
            return;
        };
        match message {
            Ok(text) => {
                let _ = refresh_ui(&ui);
                ui.set_status_error(false);
                ui.set_status_text(text.into());
            }
            Err(error) => {
                ui.set_status_error(true);
                ui.set_status_text(error.to_string().into());
            }
        }
    });

    let weak = ui.as_weak();
    ui.on_update_hotkey(move |index, name, enabled, toggle| {
        run_ui_action(&weak, "Hotkey updated.", |runtime| {
            runtime.mutate_config(|config| {
                if let Some(binding) = config.hotkeys.get_mut(index as usize) {
                    binding.name = name.to_string();
                    binding.enabled = enabled;
                    binding.toggle = toggle;
                }
            })
        });
    });

    let weak = ui.as_weak();
    ui.on_record_shortcut(move |text, ctrl, alt, shift, meta| {
        let hotkey_index = weak.upgrade().map_or(-1, |ui| ui.get_selected_hotkey());
        let Some(shortcut) = build_shortcut(text.as_str(), ctrl, alt, shift, meta) else {
            if let Some(ui) = weak.upgrade() {
                ui.set_status_error(true);
                ui.set_status_text("Unsupported key for a shortcut.".into());
            }
            return;
        };
        run_ui_action(&weak, "Shortcut recorded.", move |runtime| {
            runtime.mutate_config(move |config| {
                if let Some(binding) = config.hotkeys.get_mut(hotkey_index as usize) {
                    binding.shortcut = shortcut;
                }
            })
        });
    });

    let weak = ui.as_weak();
    ui.on_select_action(move |index| {
        if let Some(ui) = weak.upgrade() {
            select_action(&ui, index);
        }
    });

    let weak = ui.as_weak();
    ui.on_add_action(move |executable| {
        let hotkey_index = weak.upgrade().map_or(-1, |ui| ui.get_selected_hotkey());
        run_ui_action(&weak, "Action added.", |runtime| {
            runtime.mutate_config(|config| {
                if let Some(binding) = config.hotkeys.get_mut(hotkey_index as usize) {
                    let target = if executable.is_empty() {
                        Target::Foreground
                    } else {
                        Target::Process {
                            executable: executable.to_string(),
                        }
                    };
                    binding.actions.push(Action {
                        target,
                        operation: Operation::Set { level: 0.5 },
                    });
                }
            })
        });
    });

    let weak = ui.as_weak();
    ui.on_remove_action(move |hotkey_index, action_index| {
        if let Some(ui) = weak.upgrade() {
            ui.set_selected_action(-1);
        }
        run_ui_action(&weak, "Action removed.", |runtime| {
            runtime.mutate_config(|config| {
                if let Some(binding) = config.hotkeys.get_mut(hotkey_index as usize)
                    && action_index >= 0
                    && (action_index as usize) < binding.actions.len()
                {
                    binding.actions.remove(action_index as usize);
                }
            })
        });
    });

    let weak = ui.as_weak();
    ui.on_update_action(
        move |hotkey_index, action_index, target, operation, value, muted| {
            run_ui_action(&weak, "Action updated.", |runtime| {
                runtime.mutate_config(|config| {
                    let Some(action) = config
                        .hotkeys
                        .get_mut(hotkey_index as usize)
                        .and_then(|binding| binding.actions.get_mut(action_index as usize))
                    else {
                        return;
                    };
                    action.target = if target.is_empty() || target == "Focused application" {
                        Target::Foreground
                    } else {
                        Target::Process {
                            executable: target.to_string(),
                        }
                    };
                    action.operation = operation_from_ui(operation, value, muted);
                })
            });
        },
    );
}

fn run_ui_action(
    weak: &slint::Weak<AppWindow>,
    success: &str,
    action: impl FnOnce(&mut Runtime) -> Result<()>,
) {
    let result = with_runtime(action);
    let Some(ui) = weak.upgrade() else {
        return;
    };
    match result {
        Ok(()) => {
            let _ = refresh_ui(&ui);
            ui.set_status_error(false);
            ui.set_status_text(success.into());
        }
        Err(error) => {
            ui.set_status_error(true);
            ui.set_status_text(error.to_string().into());
        }
    }
}

fn update_ui(weak: &slint::Weak<AppWindow>, status: &str) {
    if let Some(ui) = weak.upgrade() {
        let _ = refresh_ui(&ui);
        ui.set_status_text(status.into());
    }
}

fn refresh_ui(ui: &AppWindow) -> Result<()> {
    with_runtime(|runtime| {
        let live = runtime.engine.sessions().unwrap_or_default();
        let query = runtime.application_search.to_ascii_lowercase();
        let applications = runtime
            .applications
            .iter()
            .filter(|entry| matches_query(entry, &query))
            .map(|entry| application_row(entry, &live))
            .collect();
        let hotkeys = runtime
            .config
            .hotkeys
            .iter()
            .map(|binding| HotkeyRow {
                name: binding.name.as_str().into(),
                shortcut: binding.shortcut.as_str().into(),
                action_count: binding.actions.len() as i32,
                enabled: binding.enabled,
                toggle: binding.toggle,
            })
            .collect();
        ui.set_applications(model(applications));
        ui.set_hotkeys(model(hotkeys));

        let selected = ui.get_selected_hotkey();
        if selected < 0 || selected as usize >= runtime.config.hotkeys.len() {
            ui.set_selected_hotkey(-1);
            ui.set_selected_action(-1);
            ui.set_actions(model(Vec::new()));
        } else {
            populate_hotkey(ui, &runtime.config.hotkeys[selected as usize]);
        }
        Ok(())
    })
}

fn application_row(entry: &AppEntry, live: &[SessionInfo]) -> ApplicationRow {
    let session = live.iter().find(|session| {
        session
            .executable
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case(&entry.executable))
    });
    ApplicationRow {
        display: entry.display.as_str().into(),
        executable: entry.executable.as_str().into(),
        running: entry.running,
        level: session.map_or(-1, |session| percent(session.level)),
        muted: session.is_some_and(|session| session.muted),
    }
}

fn matches_query(entry: &AppEntry, query: &str) -> bool {
    query.is_empty()
        || entry.display.to_ascii_lowercase().contains(query)
        || entry.executable.to_ascii_lowercase().contains(query)
}

fn select_hotkey(ui: &AppWindow, index: i32) {
    ui.set_selected_hotkey(index);
    ui.set_selected_action(-1);
    let _ = refresh_ui(ui);
}

fn populate_hotkey(ui: &AppWindow, binding: &HotkeyBinding) {
    ui.set_selected_name(binding.name.as_str().into());
    ui.set_selected_shortcut(binding.shortcut.as_str().into());
    ui.set_selected_enabled(binding.enabled);
    ui.set_selected_toggle(binding.toggle);
    ui.set_actions(model(
        binding
            .actions
            .iter()
            .map(|action| ActionRow {
                target: target_name(&action.target).into(),
                operation: operation_name(action.operation),
            })
            .collect(),
    ));
}

fn select_action(ui: &AppWindow, index: i32) {
    ui.set_selected_action(index);
    let hotkey = ui.get_selected_hotkey();
    let _ = with_runtime(|runtime| {
        let Some(action) = runtime
            .config
            .hotkeys
            .get(hotkey as usize)
            .and_then(|binding| binding.actions.get(index as usize))
        else {
            return Ok(());
        };
        ui.set_selected_target(target_name(&action.target).into());
        let (operation, value) = operation_values(action.operation);
        ui.set_selected_operation(operation);
        ui.set_selected_value(value);
        ui.set_selected_muted(matches!(action.operation, Operation::Mute { muted: true }));
        Ok(())
    });
}

fn target_name(target: &Target) -> &str {
    match target {
        Target::Foreground => "Focused application",
        Target::Process { executable } => executable,
    }
}

fn operation_name(operation: Operation) -> SharedString {
    match operation {
        Operation::Set { level } => format!("Set level to {}%", percent(level)).into(),
        Operation::Adjust { delta } => format!("Adjust level by {}%", percent(delta)).into(),
        Operation::Mute { muted: true } => "Mute".into(),
        Operation::Mute { muted: false } => "Unmute".into(),
    }
}

fn operation_values(operation: Operation) -> (i32, i32) {
    match operation {
        Operation::Set { level } => (0, percent(level)),
        Operation::Adjust { delta } => (1, percent(delta)),
        Operation::Mute { .. } => (2, 50),
    }
}

fn operation_from_ui(operation: i32, value: i32, muted: bool) -> Operation {
    let level = value as f32 / 100.0;
    match operation {
        1 => Operation::Adjust { delta: level },
        2 => Operation::Mute { muted },
        _ => Operation::Set { level },
    }
}

fn percent(value: f32) -> i32 {
    (value * 100.0).round() as i32
}

fn describe_outcome(binding: &HotkeyBinding, outcome: crate::audio::ApplyOutcome) -> String {
    if outcome.affected == 0 {
        return format!(
            "{}: no matching audio session. Is the target app playing sound?",
            binding.name
        );
    }
    let noun = if outcome.affected == 1 { "app" } else { "apps" };
    if outcome.restored {
        format!("{} restored {} {noun}.", binding.name, outcome.affected)
    } else {
        format!("{} applied to {} {noun}.", binding.name, outcome.affected)
    }
}

fn build_shortcut(text: &str, ctrl: bool, alt: bool, shift: bool, meta: bool) -> Option<String> {
    let token = key_token(text)?;
    let mut shortcut = String::new();
    if ctrl {
        shortcut.push_str("Ctrl+");
    }
    if shift {
        shortcut.push_str("Shift+");
    }
    if alt {
        shortcut.push_str("Alt+");
    }
    if meta {
        shortcut.push_str("Super+");
    }
    shortcut.push_str(&token);
    Some(shortcut)
}

fn key_token(text: &str) -> Option<String> {
    let character = text.chars().next()?;
    let named = match character {
        '\u{F700}' => Some("Up"),
        '\u{F701}' => Some("Down"),
        '\u{F702}' => Some("Left"),
        '\u{F703}' => Some("Right"),
        '\u{0020}' => Some("Space"),
        '\u{0008}' => Some("Backspace"),
        '\u{0009}' => Some("Tab"),
        '\u{000a}' | '\u{000d}' => Some("Enter"),
        '\u{001b}' => Some("Escape"),
        '\u{007f}' => Some("Delete"),
        '\u{F727}' => Some("Insert"),
        '\u{F729}' => Some("Home"),
        '\u{F72B}' => Some("End"),
        '\u{F72C}' => Some("PageUp"),
        '\u{F72D}' => Some("PageDown"),
        _ => None,
    };
    if let Some(named) = named {
        return Some(named.to_owned());
    }

    if ('\u{F704}'..='\u{F71B}').contains(&character) {
        let index = character as u32 - 0xF704 + 1;
        return Some(format!("F{index}"));
    }

    if character.is_ascii_alphanumeric() {
        return Some(character.to_ascii_uppercase().to_string());
    }

    if "`-=[]\\;',./".contains(character) {
        return Some(character.to_string());
    }

    None
}

fn next_shortcut(config: &Config) -> String {
    ('A'..='Z')
        .map(|key| format!("Ctrl+Alt+{key}"))
        .find(|candidate| {
            config
                .hotkeys
                .iter()
                .all(|binding| !binding.shortcut.eq_ignore_ascii_case(candidate))
        })
        .unwrap_or_else(|| format!("Ctrl+Shift+F{}", config.hotkeys.len() % 12 + 1))
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

#[cfg(windows)]
fn spawn_event_pump(
    ui: &AppWindow,
    open_id: tray_icon::menu::MenuId,
    quit_id: tray_icon::menu::MenuId,
) -> slint::Timer {
    use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
    use std::time::Duration;
    use tray_icon::menu::MenuEvent;

    let weak = ui.as_weak();
    let timer = slint::Timer::default();
    // The callback runs on the Slint event loop thread, so it can touch the
    // thread-local runtime and the UI directly without invoke_from_event_loop.
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                if event.state == HotKeyState::Pressed {
                    dispatch_hotkey(&weak, event.id);
                }
            }
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if event.id == open_id {
                    if let Some(ui) = weak.upgrade() {
                        let _ = ui.show();
                    }
                } else if event.id == quit_id {
                    let _ = slint::quit_event_loop();
                }
            }
        },
    );
    timer
}

#[cfg(windows)]
fn dispatch_hotkey(weak: &slint::Weak<AppWindow>, id: u32) {
    let Some(ui) = weak.upgrade() else {
        return;
    };
    match with_runtime(|runtime| runtime.handle_hotkey(id)) {
        Ok(message) if message.is_empty() => {}
        Ok(message) => {
            let _ = refresh_ui(&ui);
            ui.set_status_error(false);
            ui.set_status_text(message.into());
        }
        Err(error) => {
            ui.set_status_error(true);
            ui.set_status_text(error.to_string().into());
        }
    }
}

#[cfg(windows)]
fn create_tray(
    _ui: &AppWindow,
) -> Result<(
    tray_icon::TrayIcon,
    tray_icon::menu::MenuId,
    tray_icon::menu::MenuId,
)> {
    use tray_icon::TrayIconBuilder;
    use tray_icon::menu::{Menu, MenuItem};

    let open = MenuItem::with_id("open", "Open VOLE", true, None);
    let quit = MenuItem::with_id("quit", "Quit", true, None);
    let open_id = open.id().clone();
    let quit_id = quit.id().clone();
    let menu = Menu::with_items(&[&open, &quit]).context("failed to create tray menu")?;
    let icon = vole_icon()?;
    let tray = TrayIconBuilder::new()
        .with_tooltip("VOLE")
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .build()
        .context("failed to create tray icon")?;

    Ok((tray, open_id, quit_id))
}

#[cfg(windows)]
fn vole_icon() -> Result<tray_icon::Icon> {
    const SIZE: u32 = 32;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let v = (6..12).contains(&x) && (6..20).contains(&y)
                || (21..27).contains(&x) && (6..20).contains(&y)
                || (18..25).contains(&y) && (9..24).contains(&x);
            rgba.extend_from_slice(if v {
                &[16, 13, 24, 255]
            } else {
                &[157, 108, 255, 255]
            });
        }
    }
    tray_icon::Icon::from_rgba(rgba, SIZE, SIZE).context("failed to create VOLE icon")
}
