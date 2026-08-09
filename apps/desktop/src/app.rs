use std::cell::RefCell;
use std::collections::HashMap;
use std::str::FromStr;

use anyhow::{Context, Result};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use slint::ComponentHandle;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIconBuilder};

use crate::AppWindow;
use crate::audio::AudioController;
use crate::config::{Config, ConfigStore};

thread_local! {
    static RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) };
}

pub fn run() -> Result<()> {
    let ui = AppWindow::new().context("failed to create VOLE window")?;
    let store = ConfigStore::new()?;
    let config = store.load()?;
    let mut runtime = Runtime::new(store, config)?;
    runtime.register_hotkeys()?;
    ui.set_config_text(runtime.config_json()?.into());
    RUNTIME.with(|slot| slot.replace(Some(runtime)));

    bind_ui(&ui);
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
}

impl Runtime {
    fn new(store: ConfigStore, config: Config) -> Result<Self> {
        Ok(Self {
            store,
            config,
            hotkeys: GlobalHotKeyManager::new().context("failed to initialize global hotkeys")?,
            bindings: HashMap::new(),
            audio: AudioController::new()?,
        })
    }

    fn config_json(&self) -> Result<String> {
        serde_json::to_string_pretty(&self.config).context("failed to display config")
    }

    fn replace_config(&mut self, text: &str) -> Result<()> {
        let config: Config = serde_json::from_str(text).context("invalid configuration JSON")?;
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
    ui.on_save(move |text| {
        let result = with_runtime(|runtime| runtime.replace_config(text.as_str()));
        let Some(ui) = weak.upgrade() else {
            return;
        };
        match result {
            Ok(()) => {
                ui.set_status_error(false);
                ui.set_status_text("Saved. Hotkeys are active.".into());
            }
            Err(error) => {
                ui.set_status_error(true);
                ui.set_status_text(error.to_string().into());
            }
        }
    });

    let weak = ui.as_weak();
    ui.on_reset(move || {
        let result = with_runtime(|runtime| runtime.config_json());
        let Some(ui) = weak.upgrade() else {
            return;
        };
        match result {
            Ok(text) => {
                ui.set_config_text(text.into());
                ui.set_status_error(false);
                ui.set_status_text("Unsaved edits reset.".into());
            }
            Err(error) => {
                ui.set_status_error(true);
                ui.set_status_text(error.to_string().into());
            }
        }
    });
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
