//! Persisted user preferences. Stored at $XDG_CONFIG_HOME/byt/config.json.

use std::path::PathBuf;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub quit_on_switch: bool,
}

fn config_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", "byt").map(|d| d.config_dir().join("config.json"))
}

pub fn load() -> Config {
    let Some(path) = config_path() else {
        return Config::default();
    };
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(config: &Config) {
    let Some(path) = config_path() else { return };
    if let Some(parent) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            tracing::warn!(?err, path = %parent.display(), "could not create config dir");
            return;
        }
    }
    match serde_json::to_string_pretty(config) {
        Ok(s) => {
            if let Err(err) = std::fs::write(&path, s) {
                tracing::warn!(?err, path = %path.display(), "could not write config");
            }
        }
        Err(err) => tracing::warn!(?err, "could not serialise config"),
    }
}
