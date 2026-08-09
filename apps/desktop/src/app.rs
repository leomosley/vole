use std::cell::RefCell;
use std::collections::HashMap;
use std::str::FromStr;

use anyhow::{Context, Result};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIconBuilder};

use crate::audio::{AudioApplication, AudioController};
use crate::config::{Action, Config, ConfigStore, HotkeyBinding, Operation, Target};
use crate::{ActionRow, AppWindow, ApplicationRow, HotkeyRow};

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
    bind_hotkeys(&ui);
    let tray = create_tray(&ui)?;
    ui.window()
        .on_close_requested(|| slint::CloseRequestResponse::HideWindow);

    ui.show().context("failed to show VOLE window")?;
    let result = slint::run_event_loop_until_quit().context("VOLE event loop failed");
    GlobalHotKeyEvent::set_event_handler::<fn(GlobalHotKeyEvent)>(None);
    MenuEvent::set_event_handler::<fn(MenuEvent)>(None);
    drop(tray);
    RUNTIME.with(|slot| slot.replace(None));
    result
}

struct Runtime {
    store: ConfigStore,
    config: Config,
    hotkeys: GlobalHotKeyManager,
    bindings: HashMap<u32, usize>,
    audio: AudioController,
    applications: Vec<AudioApplication>,
    application_search: String,
}

impl Runtime {
    fn new(store: ConfigStore, config: Config) -> Result<Self> {
        Ok(Self {
            store,
            config,
            hotkeys: GlobalHotKeyManager::new().context("failed to initialize global hotkeys")?,
            bindings: HashMap::new(),
            audio: AudioController::new()?,
            applications: Vec::new(),
            application_search: String::new(),
        })
    }

    fn refresh_applications(&mut self) -> Result<()> {
        self.applications = self.audio.applications()?;
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

    fn register_hotkeys(&mut self) -> Result<()> {
        for (index, binding) in self.config.hotkeys.iter().enumerate() {
            if !binding.enabled {
                continue;
            }
            let hotkey = HotKey::from_str(&binding.shortcut)
                .with_context(|| format!("invalid shortcut `{}`", binding.shortcut))?;
            self.hotkeys
                .register(hotkey)
                .with_context(|| format!("shortcut `{}` is unavailable", binding.shortcut))?;
            self.bindings.insert(hotkey.id(), index);
        }
        Ok(())
    }

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

    fn handle_hotkey(&mut self, id: u32) -> Result<()> {
        let Some(&index) = self.bindings.get(&id) else {
            return Ok(());
        };
        let binding = &self.config.hotkeys[index];
        self.audio.apply(&binding.id, &binding.actions)
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
        run_ui_action(&weak, "Active audio refreshed.", |runtime| {
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
                    actions: vec![Action {
                        target: Target::Foreground,
                        operation: Operation::ToggleDuck { level: 0.2 },
                    }],
                });
            })
        });
    });

    let weak = ui.as_weak();
    ui.on_remove_hotkey(move |index| {
        run_ui_action(&weak, "Hotkey removed.", |runtime| {
            runtime.mutate_config(|config| {
                if index >= 0 && (index as usize) < config.hotkeys.len() {
                    config.hotkeys.remove(index as usize);
                }
            })
        });
    });

    let weak = ui.as_weak();
    ui.on_update_hotkey(move |index, name, shortcut, enabled| {
        run_ui_action(&weak, "Hotkey updated.", |runtime| {
            runtime.mutate_config(|config| {
                if let Some(binding) = config.hotkeys.get_mut(index as usize) {
                    binding.name = name.to_string();
                    binding.shortcut = shortcut.to_string();
                    binding.enabled = enabled;
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
        move |hotkey_index, action_index, target, operation, value| {
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
                    action.operation = operation_from_ui(operation, value);
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
        let query = runtime.application_search.to_ascii_lowercase();
        let applications = runtime
            .applications
            .iter()
            .filter(|application| application.executable.to_ascii_lowercase().contains(&query))
            .map(|application| ApplicationRow {
                name: application.executable.as_str().into(),
            })
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

fn select_hotkey(ui: &AppWindow, index: i32) {
    ui.set_selected_hotkey(index);
    ui.set_selected_action(-1);
    let _ = refresh_ui(ui);
}

fn populate_hotkey(ui: &AppWindow, binding: &HotkeyBinding) {
    ui.set_selected_name(binding.name.as_str().into());
    ui.set_selected_shortcut(binding.shortcut.as_str().into());
    ui.set_selected_enabled(binding.enabled);
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
    let (kind, value) = operation_values(operation);
    match kind {
        0 => format!("Set level to {value}%").into(),
        1 => format!("Adjust level by {value}%").into(),
        2 => "Toggle mute".into(),
        _ => format!("Toggle duck at {value}%").into(),
    }
}

fn operation_values(operation: Operation) -> (i32, i32) {
    match operation {
        Operation::Set { level } => (0, percent(level)),
        Operation::Adjust { delta } => (1, percent(delta)),
        Operation::ToggleMute => (2, 0),
        Operation::ToggleDuck { level } => (3, percent(level)),
    }
}

fn operation_from_ui(operation: i32, value: i32) -> Operation {
    let level = value as f32 / 100.0;
    match operation {
        0 => Operation::Set { level },
        1 => Operation::Adjust { delta: level },
        2 => Operation::ToggleMute,
        _ => Operation::ToggleDuck { level },
    }
}

fn percent(value: f32) -> i32 {
    (value * 100.0).round() as i32
}

fn next_shortcut(config: &Config) -> String {
    (1..=12)
        .map(|key| format!("Ctrl+Alt+F{key}"))
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

fn bind_hotkeys(ui: &AppWindow) {
    let weak = ui.as_weak();
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.state != HotKeyState::Pressed {
            return;
        }

        let weak = weak.clone();
        let id = event.id;
        let _ = slint::invoke_from_event_loop(move || {
            if let Err(error) = with_runtime(|runtime| runtime.handle_hotkey(id))
                && let Some(ui) = weak.upgrade()
            {
                ui.set_status_error(true);
                ui.set_status_text(error.to_string().into());
            }
        });
    }));
}

fn create_tray(ui: &AppWindow) -> Result<tray_icon::TrayIcon> {
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

    let weak = ui.as_weak();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id == open_id {
            let weak = weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    let _ = ui.show();
                }
            });
        } else if event.id == quit_id {
            let _ = slint::invoke_from_event_loop(|| {
                let _ = slint::quit_event_loop();
            });
        }
    }));

    Ok(tray)
}

fn vole_icon() -> Result<Icon> {
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
    Icon::from_rgba(rgba, SIZE, SIZE).context("failed to create VOLE icon")
}
