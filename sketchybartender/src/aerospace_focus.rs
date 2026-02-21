//! Workspace focus helper: substitute for `aerospace focus <workspace>` that
//! also ensures the configured app is running in the workspace.
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

/// Run `aerospace workspace <workspace>`
pub fn aerospace_focus(workspace: &str) -> bool {
    let status = Command::new("aerospace")
        .args(["workspace", workspace])
        .status();

    match status {
        Ok(s) => s.success(),
        Err(e) => {
            eprintln!("[aerospace-focus] Failed to run aerospace workspace: {}", e);
            false
        }
    }
}

/// Return the list of app-bundle-ids currently open in the workspace
pub fn list_workspace_bundle_ids(workspace: &str) -> Vec<String> {
    let output = Command::new("aerospace")
        .args([
            "list-windows",
            "--workspace",
            workspace,
            "--format",
            "%{app-bundle-id}",
        ])
        .output();

    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        Ok(o) => {
            eprintln!(
                "[aerospace-focus] aerospace list-windows failed: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            Vec::new()
        }
        Err(e) => {
            eprintln!("[aerospace-focus] Failed to run aerospace list-windows: {}", e);
            Vec::new()
        }
    }
}

/// Launch the configured app for `workspace` if it isn't already running there.
/// Does NOT call `aerospace workspace` — use this when aerospace has already
/// performed the focus (e.g. from an exec-on-workspace-change callback).
pub fn ensure_workspace_app(workspace: &str) {
    let config = load_config();
    let apps = match config.get(workspace) {
        Some(c) => c,
        None => return,
    };

    let bundle_ids = list_workspace_bundle_ids(workspace);

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
pub fn focus_workspace(workspace: &str) {
    if !aerospace_focus(workspace) {
        eprintln!(
            "[aerospace-focus] Warning: aerospace workspace returned non-zero for '{}'",
            workspace
        );
    }
    ensure_workspace_app(workspace);
}
