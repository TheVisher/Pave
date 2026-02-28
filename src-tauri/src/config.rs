use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaveConfig {
    #[serde(default = "default_gap_size")]
    pub gap_size: u32,
    #[serde(default)]
    pub excluded_monitors: Vec<String>,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub corner_radius: Option<u32>,
}

fn default_gap_size() -> u32 {
    15
}

impl Default for PaveConfig {
    fn default() -> Self {
        Self {
            gap_size: 15,
            excluded_monitors: Vec::new(),
            autostart: false,
            corner_radius: None,
        }
    }
}

impl PaveConfig {
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("pave")
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if path.exists() {
            match fs::read_to_string(&path) {
                Ok(content) => match toml::from_str(&content) {
                    Ok(config) => return config,
                    Err(e) => log::warn!("Failed to parse config: {e}, using defaults"),
                },
                Err(e) => log::warn!("Failed to read config: {e}, using defaults"),
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<(), String> {
        let dir = Self::config_dir();
        fs::create_dir_all(&dir).map_err(|e| format!("Failed to create config dir: {e}"))?;
        let content =
            toml::to_string_pretty(self).map_err(|e| format!("Failed to serialize config: {e}"))?;
        fs::write(Self::config_path(), content)
            .map_err(|e| format!("Failed to write config: {e}"))?;
        Ok(())
    }

    pub fn is_first_run() -> bool {
        !Self::config_path().exists()
    }
}
