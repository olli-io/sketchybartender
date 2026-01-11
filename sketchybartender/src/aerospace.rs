use std::collections::HashMap;
use std::process::Command;
use serde::Deserialize;
use crate::icon_map::get_icon;

/// Information about a single window from aerospace
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct WindowInfo {
    pub app_name: String,
    pub workspace: String,
    pub workspace_is_focused: bool,
    #[allow(dead_code)] // May be used in the future
    pub workspace_is_visible: bool,
    /// Sketchybar display ID (monitor-appkit-nsscreen-screens-id from aerospace)
    #[serde(rename = "monitor-appkit-nsscreen-screens-id")]
    pub display_id: u32,
}

/// Information about a workspace
#[derive(Debug, Clone, Default)]
pub struct WorkspaceInfo {
    #[allow(dead_code)] // Used in tests
    pub id: String,
    pub apps: Vec<String>,
    pub icons: String,
    #[allow(dead_code)] // Used in tests
    pub is_focused: bool,
    /// Sketchybar display ID (directly from aerospace's monitor-appkit-nsscreen-screens-id)
    pub display_id: u32,
}

/// Get all windows using aerospace's JSON API
/// This single command provides all the information we need about windows, workspaces, and displays
pub fn get_windows() -> Vec<WindowInfo> {
    let output = match Command::new("aerospace")
        .args([
            "list-windows",
            "--all",
            "--format",
            "%{app-name}%{workspace}%{workspace-is-focused}%{workspace-is-visible}%{monitor-appkit-nsscreen-screens-id}",
            "--json"
        ])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    if !output.status.success() {
        return Vec::new();
    }

    // Parse JSON using serde
    serde_json::from_slice(&output.stdout).unwrap_or_default()
}

/// Get the currently focused app
pub fn get_focused_app() -> Option<String> {
    let output = Command::new("aerospace")
        .args(["list-windows", "--focused", "--format", "%{app-name}"])
        .output()
        .ok()?;

    if output.status.success() {
        let app = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !app.is_empty() {
            return Some(app);
        }
    }
    None
}

/// Information about the focused workspace from aerospace
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct FocusedWorkspaceInfo {
    pub workspace: String,
    pub workspace_is_focused: bool,
    pub workspace_is_visible: bool,
    #[serde(rename = "monitor-appkit-nsscreen-screens-id")]
    pub display_id: u32,
}

/// Get the currently focused workspace
/// This is used as a fallback when no windows are open in the focused workspace
pub fn get_focused_workspace() -> Option<FocusedWorkspaceInfo> {
    let output = Command::new("aerospace")
        .args([
            "list-workspaces",
            "--focused",
            "--format",
            "%{workspace}%{workspace-is-focused}%{workspace-is-visible}%{monitor-appkit-nsscreen-screens-id}",
            "--json"
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    // Parse JSON - returns an array with a single element
    let workspaces: Vec<FocusedWorkspaceInfo> = serde_json::from_slice(&output.stdout).ok()?;
    workspaces.into_iter().next()
}

/// Get workspace information for all workspaces
///
/// # Arguments
/// * `show_all_windows` - If true, show an icon for each window. If false, show one icon per app.
pub fn get_workspace_infos(show_all_windows: bool) -> HashMap<String, WorkspaceInfo> {
    // Use the new JSON API to get all window information in one call
    // Retry if window list seems stale
    let mut windows = get_windows();
    let initial_window_count = windows.len();

    // Retry mechanism to handle aerospace state updates
    // Sometimes aerospace hasn't finished updating when we query, especially after move-node-to-workspace
    let mut retry_count = 0;
    let max_retries = 2;

    while retry_count < max_retries {
        let needs_retry = if retry_count == 0 && initial_window_count > 0 && windows.is_empty() {
            // We had windows before but now have none - might be mid-update
            eprintln!("[AEROSPACE] Warning: all windows disappeared, possible stale data (retry {}/{})", retry_count + 1, max_retries);
            true
        } else if windows.iter().any(|w| w.workspace_is_focused) {
            // We found a focused workspace, we're good
            false
        } else if !windows.is_empty() {
            // We have windows but none are focused - might be mid-update
            eprintln!("[AEROSPACE] Warning: no focused workspace found (retry {}/{})", retry_count + 1, max_retries);
            true
        } else {
            false
        };

        if !needs_retry {
            break;
        }

        // Exponential backoff: 20ms, then 40ms
        let delay_ms = 20 * (retry_count + 1) as u64;
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));

        windows = get_windows();
        retry_count += 1;
    }

    // Group windows by workspace
    let mut workspace_data: HashMap<String, (Vec<String>, bool, u32)> = HashMap::new();

    for window in windows {
        workspace_data
            .entry(window.workspace.clone())
            .and_modify(|(apps, is_focused, _display_id)| {
                apps.push(window.app_name.clone());
                *is_focused = *is_focused || window.workspace_is_focused;
            })
            .or_insert((
                vec![window.app_name],
                window.workspace_is_focused,
                window.display_id,
            ));
    }

    // If no workspace is marked as focused, query aerospace for the focused workspace
    // This handles the case where the focused workspace has no windows
    let has_focused = workspace_data.values().any(|(_, is_focused, _)| *is_focused);
    if !has_focused {
        if let Some(focused) = get_focused_workspace() {
            eprintln!("[AEROSPACE] No focused workspace in windows, adding empty focused workspace: {}", focused.workspace);
            workspace_data.insert(
                focused.workspace.clone(),
                (Vec::new(), true, focused.display_id),
            );
        }
    }

    // Build workspace infos
    let mut result = HashMap::new();

    for (ws_id, (apps, is_focused, display_id)) in workspace_data {
        // Build icons string
        let icons: String = if show_all_windows {
            // Show an icon for each window
            apps
                .iter()
                .map(|app| format!("{}", get_icon(app)))
                .collect()
        } else {
            // Show one icon per unique app
            let mut unique_apps: Vec<String> = apps.clone();
            unique_apps.sort();
            unique_apps.dedup();
            unique_apps
                .iter()
                .map(|app| format!("{}", get_icon(app)))
                .collect()
        };

        result.insert(
            ws_id.clone(),
            WorkspaceInfo {
                id: ws_id.clone(),
                apps,
                icons: icons.trim_end().to_string(),
                is_focused,
                display_id,
            },
        );
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_infos_structure() {
        // This test verifies the structure without requiring aerospace
        let mut info = WorkspaceInfo::default();
        info.id = "1".to_string();
        info.apps = vec!["Safari".to_string(), "Cursor".to_string()];
        info.icons = ":safari: :cursor:".to_string();
        info.is_focused = true;

        assert_eq!(info.apps.len(), 2);
        assert!(info.is_focused);
    }
}
