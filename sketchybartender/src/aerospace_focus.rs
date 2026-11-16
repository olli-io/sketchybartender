//! Workspace focus helper: substitute for `aerospace focus <workspace>` that
//! also ensures the configured app is running in the workspace. It works with
//! each window manager that `wm` supports; the config path keeps its AeroSpace
//! name for compatibility. With Rift, the keys are workspace numbers ("1", "2", ...).
//!
//! Config file: $XDG_CONFIG_HOME/aerospace/aerospace-workspaces.json
//!              (or ~/.config/aerospace/aerospace-workspaces.json)
//!
//! Config structure:
//! {
//!   "<workspace>": [
//!     { "app-bundle-id": "com.example.App", "start-cmd": "open -a \"App Name\"" },
//!     { "app-bundle-id": "com.example.Other", "start-cmd": "open -a \"Other App\"" }
//!   ]
//! }

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::config::WindowManager;
use crate::wm;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct WorkspaceConfig {
    pub app_bundle_id: String,
    pub start_cmd: String,
}

pub type AerospaceFocusConfig = HashMap<String, Vec<WorkspaceConfig>>;

pub fn get_config_path() -> PathBuf {
    let config_dir = env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = env::var("HOME").expect("HOME not set");
            PathBuf::from(home).join(".config")
        });
    config_dir.join("aerospace").join("aerospace-workspaces.json")
}

pub fn load_config() -> AerospaceFocusConfig {
    let path = get_config_path();

    if !path.exists() {
        // Create an example config on first run
        let mut example = HashMap::new();
        example.insert(
            "example-workspace".to_string(),
            vec![WorkspaceConfig {
                app_bundle_id: "com.example.App".to_string(),
                start_cmd: "open -a \"App Name\"".to_string(),
            }],
        );
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&example) {
            let _ = fs::write(&path, json);
        }
        eprintln!(
            "[aerospace-focus] Created example config at {:?}. Edit it to configure your workspaces.",
            path
        );
        return HashMap::new();
    }

    let data = match fs::read_to_string(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[aerospace-focus] Failed to read config {:?}: {}", path, e);
            return HashMap::new();
        }
    };

    match serde_json::from_str(&data) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("[aerospace-focus] Failed to parse config {:?}: {}", path, e);
            HashMap::new()
        }
    }
}

/// Launch the configured app for `workspace` if it isn't already running there.
/// Does NOT focus the workspace — use this when the window manager has already
/// performed the focus (e.g. from an exec-on-workspace-change callback).
pub fn ensure_workspace_app(wm: WindowManager, workspace: &str) {
    let config = load_config();
    let apps = match config.get(workspace) {
        Some(c) => c,
        None => return,
    };

    let bundle_ids = wm::workspace_bundle_ids(wm, workspace);

    for app in apps {
        let app_running = bundle_ids
            .iter()
            .any(|id| id.eq_ignore_ascii_case(&app.app_bundle_id));

        if app_running {
            eprintln!(
                "[aerospace-focus] App '{}' already running in workspace '{}'",
                app.app_bundle_id, workspace
            );
            continue;
        }

        eprintln!(
            "[aerospace-focus] App '{}' not running in workspace '{}', launching: {}",
            app.app_bundle_id, workspace, app.start_cmd
        );
        let status = Command::new("sh")
            .args(["-c", &app.start_cmd])
            .status();

        match status {
            Ok(s) if s.success() => {}
            Ok(s) => eprintln!("[aerospace-focus] start-cmd exited with status: {}", s),
            Err(e) => eprintln!("[aerospace-focus] Failed to execute start-cmd: {}", e),
        }
    }
}

/// Focus the workspace and launch the configured app if it isn't already running there.
pub fn focus_workspace(wm: WindowManager, workspace: &str) {
    if !wm::focus_workspace(wm, workspace) {
        eprintln!(
            "[aerospace-focus] Warning: focusing workspace '{}' failed",
            workspace
        );
    }
    ensure_workspace_app(wm, workspace);
}
