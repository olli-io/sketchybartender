//! Window-manager facade. Each function routes to the integration that the
//! `window_manager` config key selects, so the handlers do not need to know
//! which window manager runs.

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use crate::config::WindowManager;
use crate::handlers::DaemonState;
use crate::{aerospace, aerospace_events, aerospace_socket, rift, rift_events};

/// Information about a workspace
#[derive(Debug, Clone, Default)]
pub struct WorkspaceInfo {
    #[allow(dead_code)] // Used in tests
    pub id: String,
    /// App name of each window (one entry for each window)
    pub apps: Vec<String>,
    #[allow(dead_code)] // Used in tests
    pub is_focused: bool,
    /// Sketchybar display ID (arrangement index, starts at 1)
    pub display_id: u32,
}

impl WorkspaceInfo {
    pub fn new(id: String, apps: Vec<String>, is_focused: bool, display_id: u32) -> Self {
        Self { id, apps, is_focused, display_id }
    }
}

/// Workspaces to show on the bar, keyed by workspace ID.
pub fn workspace_infos(wm: WindowManager) -> HashMap<String, WorkspaceInfo> {
    match wm {
        WindowManager::Aerospace => aerospace::get_workspace_infos(),
        WindowManager::Rift => rift::get_workspace_infos(),
    }
}

/// A fingerprint of the window set. A change tells the watcher to repaint.
/// `None` means the window manager is not available.
pub fn window_signature(wm: WindowManager) -> Option<String> {
    match wm {
        WindowManager::Aerospace => aerospace_events::window_signature(),
        WindowManager::Rift => rift::window_signature(),
    }
}

/// Switch to the workspace. Returns `false` if the window manager failed.
pub fn focus_workspace(wm: WindowManager, workspace: &str) -> bool {
    let result = match wm {
        WindowManager::Aerospace => aerospace_socket::run(&["workspace", workspace])
            .map(|_| ())
            .map_err(|e| e.to_string()),
        WindowManager::Rift => rift::focus_workspace(workspace),
    };
    if let Err(e) = &result {
        eprintln!("[wm] Failed to focus workspace '{}': {}", workspace, e);
    }
    result.is_ok()
}

/// The bundle IDs of the windows in the workspace.
pub fn workspace_bundle_ids(wm: WindowManager, workspace: &str) -> Vec<String> {
    match wm {
        WindowManager::Aerospace => aerospace_socket::run(&[
            "list-windows",
            "--workspace",
            workspace,
            "--format",
            "%{app-bundle-id}",
        ])
        .map(|stdout| {
            stdout
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_else(|e| {
            eprintln!("[wm] aerospace list-windows failed: {}", e);
            Vec::new()
        }),
        WindowManager::Rift => rift::workspace_bundle_ids(workspace),
    }
}

/// Start the threads that keep the bar in sync with the window manager.
/// Returns the sender that tells the window-set watcher to check again.
pub fn spawn_events(wm: WindowManager, state: Arc<Mutex<DaemonState>>) -> Sender<()> {
    match wm {
        WindowManager::Aerospace => aerospace_events::spawn(state),
        WindowManager::Rift => rift_events::spawn(state),
    }
}
