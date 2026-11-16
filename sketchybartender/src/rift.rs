//! Queries and commands for the Rift window manager.
//!
//! Rift has no Unix socket: its IPC is a Mach bootstrap service
//! (`git.acsandmann.rift`) with JSON payloads. We use Rift's own `rift-client`
//! crate, which talks to the same service as `rift-cli`.
//!
//! Workspace IDs on the bar are the 1-based Rift workspace index ("1", "2",
//! ...), so they agree with the `workspace.N` items in sketchybarrc. In Rift's
//! default `per_display` mode, two displays can each have a workspace "1". A
//! sketchybar item can be on one display only, so the focused one wins.

use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr};

use rift_client::{DisplayData, LayoutCommand, RiftCommand, RiftMachClient, WindowData, WorkspaceData};

use crate::wm::WorkspaceInfo;

/// One display and its workspaces, as Rift reports them.
struct DisplayWorkspaces {
    /// Sketchybar display ID (arrangement index, starts at 1)
    display_id: u32,
    workspaces: Vec<WorkspaceData>,
    is_active_context: bool,
}

/// Bar ID of a Rift workspace.
fn workspace_id(ws: &WorkspaceData) -> String {
    (ws.index + 1).to_string()
}

/// Older Rift servers do not send `is_focused`. Then the active workspace on
/// the display that receives commands is the focused one.
fn is_focused(ws: &WorkspaceData, display: &DisplayWorkspaces) -> bool {
    ws.is_focused || (ws.is_active && display.is_active_context)
}

fn window_app_name(w: &WindowData) -> Option<String> {
    w.app_name.clone().or_else(|| w.bundle_id.clone())
}

/// Get the workspaces of each display. `None` if Rift is not available.
fn query_displays() -> Option<Vec<DisplayWorkspaces>> {
    let client = RiftMachClient;
    let displays = client.get_displays().ok()?;
    let arrangement = display_arrangement();

    let result = displays
        .iter()
        .enumerate()
        // Skip displays whose space Rift does not manage.
        .filter(|(_, d)| d.space.is_some() && d.is_active_space)
        .filter_map(|(pos, display)| {
            let workspaces = match client.get_workspaces_for_display(display.uuid.clone()) {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("[rift] Failed to get workspaces for display {}: {}", display.uuid, e);
                    return None;
                }
            };
            Some(DisplayWorkspaces {
                display_id: sketchybar_display_id(&arrangement, display).unwrap_or(pos as u32 + 1),
                // In global mode a workspace has one home display. Do not show it on others.
                workspaces: workspaces
                    .into_iter()
                    .filter(|ws| ws.display_uuid.as_deref().map_or(true, |u| u == display.uuid))
                    .collect(),
                is_active_context: display.is_active_context,
            })
        })
        .collect();
    Some(result)
}

/// Get workspace information for the workspaces that have windows, plus the
/// focused workspace. This agrees with what AeroSpace shows.
///
/// # Arguments
/// * `show_all_windows` - If true, show an icon for each window. If false, show one icon per app.
pub fn get_workspace_infos(show_all_windows: bool) -> HashMap<String, WorkspaceInfo> {
    let mut result: HashMap<String, WorkspaceInfo> = HashMap::new();

    for display in query_displays().unwrap_or_default() {
        for ws in &display.workspaces {
            let focused = is_focused(ws, &display);
            if ws.windows.is_empty() && !focused {
                continue;
            }

            let id = workspace_id(ws);
            if let Some(existing) = result.get(&id) {
                if existing.is_focused || !focused {
                    continue;
                }
            }

            let apps = ws.windows.iter().filter_map(window_app_name).collect();
            result.insert(
                id.clone(),
                WorkspaceInfo::new(id, apps, focused, display.display_id, show_all_windows),
            );
        }
    }

    result
}

/// A fingerprint of every window, its workspace and display, and the focused
/// workspace. The focused workspace is included because a debounced workspace
/// switch changes no window, and the watcher must still see it.
pub fn window_signature() -> Option<String> {
    let mut lines = Vec::new();
    for display in query_displays()? {
        for ws in &display.workspaces {
            if is_focused(ws, &display) {
                lines.push(format!("focused {} {}", display.display_id, ws.index));
            }
            for w in &ws.windows {
                lines.push(format!("{} {} {} {}", display.display_id, ws.index, w.id.pid, w.id.idx));
            }
        }
    }
    lines.sort_unstable();
    Some(lines.join("\n"))
}

/// The name of the frontmost app.
pub fn focused_app_name() -> Option<String> {
    RiftMachClient
        .get_applications()
        .ok()?
        .into_iter()
        .find(|app| app.is_frontmost)
        .map(|app| app.name)
}

/// The app name of a window, from the payload of a `focused_window_changed` event.
pub fn window_app_name_for(window_id: rift_client::WindowId) -> Option<String> {
    RiftMachClient
        .get_window_info(window_id)
        .ok()
        .and_then(|w| window_app_name(&w))
}

/// Parse a bar workspace ID ("1", "2", ...) into a 0-based Rift index.
fn parse_workspace(workspace: &str) -> Result<usize, String> {
    match workspace.trim().parse::<usize>() {
        Ok(n) if n >= 1 => Ok(n - 1),
        _ => Err(format!("'{}' is not a Rift workspace number (1, 2, ...)", workspace)),
    }
}

/// Switch to a workspace on the active display.
pub fn focus_workspace(workspace: &str) -> Result<(), String> {
    let index = parse_workspace(workspace)?;
    RiftMachClient
        .execute(RiftCommand::Layout(LayoutCommand::SwitchToWorkspace(index)))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Bundle IDs of the windows in a workspace on the active display.
pub fn workspace_bundle_ids(workspace: &str) -> Vec<String> {
    let Ok(index) = parse_workspace(workspace) else {
        return Vec::new();
    };
    match RiftMachClient.get_workspaces(None) {
        Ok(workspaces) => workspaces
            .into_iter()
            .filter(|ws| ws.index == index)
            .flat_map(|ws| ws.windows)
            .filter_map(|w| w.bundle_id)
            .collect(),
        Err(e) => {
            eprintln!("[rift] Failed to get workspaces: {}", e);
            Vec::new()
        }
    }
}

// --- Display arrangement -----------------------------------------------------
//
// Rift identifies a display by UUID and CGDirectDisplayID. Sketchybar's
// `display=N` uses the arrangement index: the position of the display UUID in
// `SLSCopyManagedDisplays`, starting at 1. We read the same list, so the two
// agree.

type CFTypeRef = *const c_void;

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

#[link(name = "SkyLight", kind = "framework")]
extern "C" {
    fn SLSMainConnectionID() -> i32;
    fn SLSCopyManagedDisplays(cid: i32) -> CFTypeRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(array: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> CFTypeRef;
    fn CFStringGetCString(string: CFTypeRef, buffer: *mut c_char, size: isize, encoding: u32) -> bool;
    fn CFRelease(cf: CFTypeRef);
}

/// Display UUIDs in sketchybar's arrangement order. Empty if SkyLight fails.
fn display_arrangement() -> Vec<String> {
    let mut uuids = Vec::new();
    unsafe {
        let displays = SLSCopyManagedDisplays(SLSMainConnectionID());
        if displays.is_null() {
            return uuids;
        }
        for i in 0..CFArrayGetCount(displays) {
            let uuid = CFArrayGetValueAtIndex(displays, i);
            let mut buf = [0 as c_char; 128];
            if !uuid.is_null()
                && CFStringGetCString(uuid, buf.as_mut_ptr(), buf.len() as isize, K_CF_STRING_ENCODING_UTF8)
            {
                uuids.push(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned());
            }
        }
        CFRelease(displays);
    }
    uuids
}

fn sketchybar_display_id(arrangement: &[String], display: &DisplayData) -> Option<u32> {
    arrangement
        .iter()
        .position(|uuid| uuid.eq_ignore_ascii_case(&display.uuid))
        .map(|i| i as u32 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_workspace() {
        assert_eq!(parse_workspace("1"), Ok(0));
        assert_eq!(parse_workspace("10"), Ok(9));
        assert!(parse_workspace("0").is_err());
        assert!(parse_workspace("a").is_err());
    }
}
