//! Configuration module for sketchybartender update intervals

use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;

/// Configuration for update intervals (in seconds)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Clock update interval (default: 15 seconds)
    pub clock_interval: u64,
    /// Battery update interval (default: 120 seconds)
    pub battery_interval: u64,
    /// Brew outdated check interval (default: 3600 seconds / 1 hour)
    pub brew_interval: u64,
    /// Teams notification check interval (default: 30 seconds)
    pub teams_interval: u64,
    /// Workspace background color (default: 0xfff38ba8)
    pub workspace_bg_color: String,
    /// Workspace focused label color (default: 0xff1d2021)
    pub workspace_focused_label_color: String,
    /// Workspace focused icon color (default: 0xff1d2021)
    pub workspace_focused_icon_color: String,
    /// Workspace unfocused label color (default: 0xffffffff)
    pub workspace_unfocused_label_color: String,
    /// Workspace unfocused icon color (default: 0xffffffff)
    pub workspace_unfocused_icon_color: String,
    /// Border active color (default: gradient(top_left=0xffbb60cd,bottom_right=0xffffad00))
    pub border_active_color: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            clock_interval: 15,
            battery_interval: 120,
            brew_interval: 3600,
            teams_interval: 30,
            workspace_bg_color: "0xffbb60cd".to_string(),
            workspace_focused_label_color: "0xff1d2021".to_string(),
            workspace_focused_icon_color: "0xff1d2021".to_string(),
            workspace_unfocused_label_color: "0xffffffff".to_string(),
            workspace_unfocused_icon_color: "0xffffffff".to_string(),
            border_active_color: "gradient(top_left=0xffbb60cd,bottom_right=0xffffad00)".to_string(),
        }
    }
}

impl Config {
    /// Load configuration from file or use defaults
    pub fn load() -> Self {
        let config_path = Self::get_config_path();

        if config_path.exists() {
            match Self::load_from_file(&config_path) {
                Ok(config) => config,
                Err(e) => {
                    eprintln!("Failed to load config from {:?}: {}", config_path, e);
                    eprintln!("Using default configuration");
                    Self::default()
                }
            }
        } else {
            // Create default config file
            let config = Self::default();
            if let Err(e) = config.save_to_file(&config_path) {
                eprintln!("Failed to save default config: {}", e);
            } else {
                eprintln!("Created default config at {:?}", config_path);
            }
            config
        }
    }

    /// Get the configuration file path
    fn get_config_path() -> PathBuf {
        let config_dir = env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = env::var("HOME").expect("HOME not set");
                PathBuf::from(home).join(".config")
            });

        config_dir.join("sketchybar").join("sketchybartender.json")
    }

    /// Load configuration from a file
    fn load_from_file(path: &PathBuf) -> Result<Self, String> {
        let contents = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read file: {}", e))?;

        serde_json::from_str(&contents)
            .map_err(|e| format!("Failed to parse JSON config: {}", e))
    }

    /// Save configuration to a file
    fn save_to_file(&self, path: &PathBuf) -> Result<(), String> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }

        let contents = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;

        fs::write(path, contents)
            .map_err(|e| format!("Failed to write config file: {}", e))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.clock_interval, 15);
        assert_eq!(config.battery_interval, 120);
        assert_eq!(config.brew_interval, 3600);
        assert_eq!(config.teams_interval, 30);
    }
}
