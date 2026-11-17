//! Queries and commands for the Rift window manager.
//!
//! Rift has no Unix socket: its IPC is a Mach bootstrap service
//! (`git.acsandmann.rift`) with JSON payloads. We use Rift's own `rift-client`
//! crate, which talks to the same service as `rift-cli`.
//!
//! The bar ID of a workspace is its name when the name is one of the
//! `workspace.N` items in sketchybarrc (one character, 1-9 or a-z). Otherwise
//! it is the 1-based Rift workspace index ("1", "2", ...). In Rift's default
//! `per_display` mode, two displays can each have a workspace "1". A sketchybar
//! item can be on one display only, so the focused one wins.

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

/// Bar ID of a Rift workspace: its name if sketchybarrc has an item for that
/// name, else its 1-based index.
fn workspace_id(ws: &WorkspaceData) -> String {
    if is_bar_item_name(&ws.name) {
        ws.name.clone()
    } else {
        (ws.index + 1).to_string()
    }
}

/// True if sketchybarrc has a `workspace.<name>` item: one character, 1-9 or a-z.
fn is_bar_item_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!((chars.next(), chars.next()), (Some('1'..='9' | 'a'..='z'), None))
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
pub fn get_workspace_infos() -> HashMap<String, WorkspaceInfo> {
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

            // One entry for each window, also for a window without an app name.
            let apps = ws
                .windows
                .iter()
                .map(|w| window_app_name(w).unwrap_or_default())
                .collect();
            result.insert(
                id.clone(),
                WorkspaceInfo::new(id, apps, focused, display.display_id),
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

/// Find the Rift workspace that has a bar ID (see `workspace_id`). In global
/// mode, the list has the workspaces of all displays.
fn find_workspace(workspace: &str) -> Result<WorkspaceData, String> {
    let workspace = workspace.trim();
    RiftMachClient
        .get_workspaces(None)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|ws| workspace_id(ws) == workspace)
        .ok_or_else(|| format!("'{}' is not a Rift workspace", workspace))
}

/// Switch to a workspace.
pub fn focus_workspace(workspace: &str) -> Result<(), String> {
    let index = find_workspace(workspace)?.index;
    RiftMachClient
        .execute(RiftCommand::Layout(LayoutCommand::SwitchToWorkspace(index)))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Bundle IDs of the windows in a workspace.
pub fn workspace_bundle_ids(workspace: &str) -> Vec<String> {
    match find_workspace(workspace) {
        Ok(ws) => ws.windows.into_iter().filter_map(|w| w.bundle_id).collect(),
        Err(e) => {
            eprintln!("[rift] Failed to get workspace: {}", e);
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
    fn test_is_bar_item_name() {
        assert!(is_bar_item_name("1"));
        assert!(is_bar_item_name("9"));
        assert!(is_bar_item_name("a"));
        assert!(is_bar_item_name("z"));
        assert!(!is_bar_item_name("0"));
        assert!(!is_bar_item_name("10"));
        assert!(!is_bar_item_name("A"));
        assert!(!is_bar_item_name("first"));
        assert!(!is_bar_item_name(""));
    }
}
