use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CONFIG_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub launch_on_startup: bool,
    pub hotkeys: Vec<HotkeyBinding>,
}

impl Config {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            version: CONFIG_VERSION,
            launch_on_startup: false,
            hotkeys: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.version != CONFIG_VERSION {
            return Err(ValidationError::UnsupportedVersion(self.version));
        }

        let mut ids = HashSet::with_capacity(self.hotkeys.len());
        let mut shortcuts = HashSet::with_capacity(self.hotkeys.len());

        for binding in &self.hotkeys {
            binding.validate()?;

            if !ids.insert(binding.id.as_str()) {
                return Err(ValidationError::DuplicateId(binding.id.clone()));
            }

            let normalized = binding.shortcut.to_ascii_lowercase();
            if !shortcuts.insert(normalized) {
                return Err(ValidationError::DuplicateShortcut(binding.shortcut.clone()));
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HotkeyBinding {
    pub id: String,
    pub name: String,
    pub shortcut: String,
    pub enabled: bool,
    pub actions: Vec<Action>,
}

impl HotkeyBinding {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.id.trim().is_empty() {
            return Err(ValidationError::EmptyId);
        }
        if self.name.trim().is_empty() {
            return Err(ValidationError::EmptyName(self.id.clone()));
        }
        if self.shortcut.trim().is_empty() {
            return Err(ValidationError::EmptyShortcut(self.id.clone()));
        }
        if self.actions.is_empty() {
            return Err(ValidationError::NoActions(self.id.clone()));
        }

        for action in &self.actions {
            action.validate(&self.id)?;
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub target: Target,
    pub operation: Operation,
}

impl Action {
    fn validate(&self, binding_id: &str) -> Result<(), ValidationError> {
        if let Target::Process { executable } = &self.target
            && executable.trim().is_empty()
        {
            return Err(ValidationError::EmptyExecutable(binding_id.to_owned()));
        }

        self.operation.validate(binding_id)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Target {
    Process { executable: String },
    Foreground,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Operation {
    Set { level: f32 },
    Adjust { delta: f32 },
    ToggleMute,
    ToggleDuck { level: f32 },
}

impl Operation {
    fn validate(self, binding_id: &str) -> Result<(), ValidationError> {
        match self {
            Self::Set { level } | Self::ToggleDuck { level } if !(0.0..=1.0).contains(&level) => {
                Err(ValidationError::InvalidLevel(binding_id.to_owned()))
            }
            Self::Adjust { delta } if !(-1.0..=1.0).contains(&delta) || delta == 0.0 => {
                Err(ValidationError::InvalidDelta(binding_id.to_owned()))
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum ValidationError {
    #[error("unsupported config version {0}")]
    UnsupportedVersion(u32),
    #[error("hotkey id cannot be empty")]
    EmptyId,
    #[error("hotkey `{0}` must have a name")]
    EmptyName(String),
    #[error("hotkey `{0}` must have a shortcut")]
    EmptyShortcut(String),
    #[error("hotkey `{0}` must contain at least one action")]
    NoActions(String),
    #[error("hotkey `{0}` contains an empty process executable")]
    EmptyExecutable(String),
    #[error("hotkey `{0}` contains a volume outside 0-100%")]
    InvalidLevel(String),
    #[error("hotkey `{0}` contains an invalid relative adjustment")]
    InvalidDelta(String),
    #[error("duplicate hotkey id `{0}`")]
    DuplicateId(String),
    #[error("duplicate shortcut `{0}`")]
    DuplicateShortcut(String),
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("VOLE application data directory is unavailable")]
    MissingDataDirectory,
    #[error("failed to read config at {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse config at {path}: {source}")]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("failed to serialize config: {0}")]
    Serialize(serde_json::Error),
    #[error("failed to save config at {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new() -> Result<Self, ConfigError> {
        let project_dirs =
            ProjectDirs::from("dev", "VOLE", "VOLE").ok_or(ConfigError::MissingDataDirectory)?;
        Ok(Self::at(project_dirs.config_dir().join("config.json")))
    }

    #[must_use]
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Result<Config, ConfigError> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Config::empty()),
            Err(source) => {
                return Err(ConfigError::Read {
                    path: self.path.clone(),
                    source,
                });
            }
        };

        let config: Config = serde_json::from_str(&text).map_err(|source| ConfigError::Parse {
            path: self.path.clone(),
            source,
        })?;
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, config: &Config) -> Result<(), ConfigError> {
        config.validate()?;
        let contents = serde_json::to_vec_pretty(config).map_err(ConfigError::Serialize)?;
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));

        fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
            path: self.path.clone(),
            source,
        })?;

        let temporary = self.path.with_extension("json.tmp");
        fs::write(&temporary, contents).map_err(|source| ConfigError::Write {
            path: temporary.clone(),
            source,
        })?;
        fs::rename(&temporary, &self.path).map_err(|source| ConfigError::Write {
            path: self.path.clone(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn binding() -> HotkeyBinding {
        HotkeyBinding {
            id: "hear-discord".to_owned(),
            name: "Hear Discord".to_owned(),
            shortcut: "Ctrl+Alt+D".to_owned(),
            enabled: true,
            actions: vec![Action {
                target: Target::Foreground,
                operation: Operation::ToggleDuck { level: 0.2 },
            }],
        }
    }

    fn temporary_config_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should follow Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("vole-config-{nonce}.json"))
    }

    #[test]
    fn empty_should_create_current_schema_version() {
        assert_eq!(Config::empty().version, CONFIG_VERSION);
    }

    #[test]
    fn validate_should_reject_duplicate_shortcuts_case_insensitively() {
        let mut duplicate = binding();
        duplicate.id = "second".to_owned();
        duplicate.shortcut = "ctrl+alt+d".to_owned();
        let config = Config {
            hotkeys: vec![binding(), duplicate],
            ..Config::empty()
        };

        assert!(matches!(
            config.validate(),
            Err(ValidationError::DuplicateShortcut(_))
        ));
    }

    #[test]
    fn validate_should_reject_volume_above_one() {
        let mut invalid = binding();
        invalid.actions[0].operation = Operation::Set { level: 1.1 };
        let config = Config {
            hotkeys: vec![invalid],
            ..Config::empty()
        };

        assert!(matches!(
            config.validate(),
            Err(ValidationError::InvalidLevel(_))
        ));
    }

    #[test]
    fn load_should_return_empty_config_when_file_is_missing() {
        let store = ConfigStore::at(temporary_config_path());

        assert_eq!(
            store.load().expect("missing config should be valid"),
            Config::empty()
        );
    }

    #[test]
    fn save_should_round_trip_valid_config() {
        let path = temporary_config_path();
        let store = ConfigStore::at(path.clone());
        let config = Config {
            hotkeys: vec![binding()],
            ..Config::empty()
        };

        store.save(&config).expect("config should save");
        let loaded = store.load().expect("config should load");
        let _ = fs::remove_file(path);

        assert_eq!(loaded, config);
    }
}
