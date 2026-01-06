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

/// Get the monitor ID for each workspace (1-9)
pub fn get_workspace_monitors() -> HashMap<String, u32> {
    let mut result = HashMap::new();

    let output = match Command::new("aerospace")
        .args(["list-workspaces", "--monitor", "all", "--format", "%{monitor-id}|%{workspace}"])
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
            if let Ok(monitor_id) = parts[0].parse::<u32>() {
                result.insert(parts[1].to_string(), monitor_id);
            }
        }
    }

    result
}

/// Get workspace information for all workspaces 1-9
pub fn get_workspace_infos() -> HashMap<String, WorkspaceInfo> {
    // Query all aerospace state at once to get consistent snapshot
    // Retry once if focused workspace is empty (might indicate aerospace is still updating)
    let mut focused = get_focused_workspace().unwrap_or_default();
    let mut windows = get_all_windows();
    let mut monitors = get_workspace_monitors();
    
    // If focused workspace is empty, retry once after a short delay
    if focused.is_empty() {
        eprintln!("[AEROSPACE] Warning: focused workspace empty, retrying...");
        std::thread::sleep(std::time::Duration::from_millis(20));
        focused = get_focused_workspace().unwrap_or_default();
        windows = get_all_windows();
        monitors = get_workspace_monitors();
    }

    // Group apps by workspace, using HashSet to deduplicate app names
    let mut workspace_apps: HashMap<String, HashSet<String>> = HashMap::new();

    for (ws, app) in windows {
        workspace_apps
            .entry(ws)
            .or_default()
            .insert(app);
    }

    // Build workspace infos for workspaces 1-9
    let mut result = HashMap::new();
    for i in 1..=9 {
        let id = i.to_string();
        // Convert HashSet to Vec and sort alphabetically
        let mut apps: Vec<String> = workspace_apps
            .get(&id)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default();
        apps.sort();

        // Build icons string
        let icons: String = apps
            .iter()
            .map(|app| format!("{} ", get_icon(app)))
            .collect();

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
