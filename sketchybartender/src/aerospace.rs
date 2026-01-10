use std::collections::{HashMap, HashSet};
use std::process::Command;
use crate::icon_map::get_icon;

/// Information about a workspace
#[derive(Debug, Clone, Default)]
pub struct WorkspaceInfo {
    #[allow(dead_code)] // Used in tests
    pub id: String,
    pub apps: Vec<String>,
    pub icons: String,
    #[allow(dead_code)] // Used in tests
    pub is_focused: bool,
    /// Aerospace monitor ID this workspace belongs to
    pub monitor_id: u32,
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

/// Get the currently focused workspace
pub fn get_focused_workspace() -> Option<String> {
    let output = Command::new("aerospace")
        .args(["list-workspaces", "--focused"])
        .output()
        .ok()?;

    if output.status.success() {
        let ws = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !ws.is_empty() {
            return Some(ws);
        }
    }
    None
}

/// Get all windows with their workspace and app name
pub fn get_all_windows() -> Vec<(String, String)> {
    let output = match Command::new("aerospace")
        .args(["list-windows", "--all", "--format", "%{workspace}|%{app-name}"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(2, '|').collect();
            if parts.len() == 2 {
                Some((parts[0].to_string(), parts[1].to_string()))
            } else {
                None
            }
        })
        .collect()
}

/// Get the monitor ID for each workspace
pub fn get_workspace_monitors() -> HashMap<String, u32> {
    let mut result = HashMap::new();

    let output = match Command::new("aerospace")
        .args(["list-workspaces", "--all", "--format", "%{workspace}|%{monitor-id}"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return result,
    };

    if !output.status.success() {
        return result;
    }

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let parts: Vec<&str> = line.splitn(2, '|').collect();
        if parts.len() == 2 {
            if let Ok(monitor_id) = parts[1].parse::<u32>() {
                result.insert(parts[0].to_string(), monitor_id);
            }
        }
    }

    result
}

/// Get workspace information for all workspaces
///
/// # Arguments
/// * `show_all_windows` - If true, show an icon for each window. If false, show one icon per app.
pub fn get_workspace_infos(show_all_windows: bool) -> HashMap<String, WorkspaceInfo> {
    // Query all aerospace state at once to get consistent snapshot
    // Retry if focused workspace is empty or if window list seems stale
    let mut focused = get_focused_workspace().unwrap_or_default();
    let mut windows = get_all_windows();
    let mut monitors = get_workspace_monitors();
    let initial_window_count = windows.len();

    // Retry mechanism to handle aerospace state updates
    // Sometimes aerospace hasn't finished updating when we query, especially after move-node-to-workspace
    let mut retry_count = 0;
    let max_retries = 2;

    while retry_count < max_retries {
        let needs_retry = if focused.is_empty() {
            // Focused workspace is empty - definitely need to retry
            eprintln!("[AEROSPACE] Warning: focused workspace empty (retry {}/{})", retry_count + 1, max_retries);
            true
        } else if retry_count == 0 && initial_window_count > 0 && windows.is_empty() {
            // We had windows before but now have none - might be mid-update
            eprintln!("[AEROSPACE] Warning: all windows disappeared, possible stale data (retry {}/{})", retry_count + 1, max_retries);
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

        focused = get_focused_workspace().unwrap_or_default();
        windows = get_all_windows();
        monitors = get_workspace_monitors();
        retry_count += 1;
    }

    // Group apps by workspace, keeping all windows (including multiple windows of the same app)
    let mut workspace_apps: HashMap<String, Vec<String>> = HashMap::new();

    for (ws, app) in windows {
        workspace_apps
            .entry(ws)
            .or_default()
            .push(app);
    }

    // Build workspace infos for all workspaces found
    let mut result = HashMap::new();

    // Get all workspace IDs from the monitors map and workspace_apps
    let mut all_workspace_ids: HashSet<String> = monitors.keys().cloned().collect();
    all_workspace_ids.extend(workspace_apps.keys().cloned());

    for id in all_workspace_ids {
        // Get all apps for this workspace (including duplicates for multiple windows)
        let apps: Vec<String> = workspace_apps
            .get(&id)
            .cloned()
            .unwrap_or_default();

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
            id.clone(),
            WorkspaceInfo {
                id: id.clone(),
                apps,
                icons: icons.trim_end().to_string(),
                is_focused: id == focused,
                monitor_id: monitors.get(&id).copied().unwrap_or(1),
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
