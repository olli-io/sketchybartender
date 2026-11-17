use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::aerospace_focus;
use crate::config::WindowManager;
use crate::mach_client;
use crate::providers;
use crate::wm;

/// Parse a color like "0xffbb60cd" to (r, g, b) tuple
fn parse_color(color: &str) -> Option<(u8, u8, u8)> {
    let hex = color.strip_prefix("0x")?;
    if hex.len() < 6 {
        return None;
    }
    let rgb_start = if hex.len() >= 8 { hex.len() - 6 } else { 0 };
    let r = u8::from_str_radix(&hex[rgb_start..rgb_start + 2], 16).ok()?;
    let g = u8::from_str_radix(&hex[rgb_start + 2..rgb_start + 4], 16).ok()?;
    let b = u8::from_str_radix(&hex[rgb_start + 4..rgb_start + 6], 16).ok()?;
    Some((r, g, b))
}

/// Generate a linear gradient with n steps between two colors
fn generate_gradient(start: (u8, u8, u8), end: (u8, u8, u8), steps: usize) -> Vec<String> {
    if steps <= 1 {
        return vec![format!("0xff{:02x}{:02x}{:02x}", start.0, start.1, start.2)];
    }
    (0..steps)
        .map(|i| {
            let t = i as f32 / (steps - 1) as f32;
            let r = (start.0 as f32 + (end.0 as f32 - start.0 as f32) * t) as u8;
            let g = (start.1 as f32 + (end.1 as f32 - start.1 as f32) * t) as u8;
            let b = (start.2 as f32 + (end.2 as f32 - start.2 as f32) * t) as u8;
            format!("0xff{:02x}{:02x}{:02x}", r, g, b)
        })
        .collect()
}

/// Get workspace background colors (15 steps) from config gradient fields
fn get_workspace_gradient_colors(config: &crate::config::Config) -> Vec<String> {
    if !config.workspace_gradient {
        return vec![config.workspace_gradient_start.clone(); 15];
    }
    match (
        parse_color(&config.workspace_gradient_start),
        parse_color(&config.workspace_gradient_end),
    ) {
        (Some(start), Some(end)) => generate_gradient(start, end, 15),
        _ => vec![config.workspace_gradient_start.clone(); 15],
    }
}

/// A builder for batching sketchybar commands
#[derive(Debug, Default)]
pub struct SketchybarBatch {
    args: Vec<String>,
}

impl SketchybarBatch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set properties on an item
    pub fn set(&mut self, item: &str, props: &[(&str, &str)]) -> &mut Self {
        self.args.push("--set".to_string());
        self.args.push(item.to_string());
        for (key, value) in props {
            // Quote the value if it contains spaces or special characters
            let formatted_value = if value.contains(' ') || value.is_empty() {
                format!("{}=\"{}\"", key, value)
            } else {
                format!("{}={}", key, value)
            };
            self.args.push(formatted_value);
        }
        self
    }

    /// Add animation with curve and duration
    pub fn animate(&mut self, curve: &str, duration: u32) -> &mut Self {
        self.args.push("--animate".to_string());
        self.args.push(curve.to_string());
        self.args.push(duration.to_string());
        self
    }

    /// Execute the batched commands
    pub fn execute(&self) -> Result<(), std::io::Error> {
        if self.args.is_empty() {
            return Ok(());
        }

        // Convert args to a single command string for mach port
        let command = self.args.join(" ");

        // Debug: print the command being sent
        eprintln!("[DEBUG] Sending to sketchybar: {}", command);

        // Send via mach port
        match mach_client::sketchybar(&command) {
            Ok(_) => {
                eprintln!("[DEBUG] Command sent successfully");
                Ok(())
            }
            Err(e) => {
                eprintln!("[DEBUG] Command failed: {}", e);
                Err(std::io::Error::new(std::io::ErrorKind::Other, format!("sketchybar mach command failed: {}", e)))
            }
        }
    }
}

/// Convenience function to set properties on a single item
fn set_item(item: &str, props: &[(&str, &str)]) -> Result<(), std::io::Error> {
    let mut batch = SketchybarBatch::new();
    batch.set(item, props);
    batch.execute()
}

/// Update the clock item
fn update_clock(time: &str) -> Result<(), std::io::Error> {
    set_item("clock", &[("label", time)])
}

/// Update the battery item
fn update_battery(icon: &str, icon_color: &str, label_color: &str, percentage: u8) -> Result<(), std::io::Error> {
    set_item("battery", &[
        ("icon", icon),
        ("icon.color", icon_color),
        ("label.color", label_color),
        ("label", &format!("{}%", percentage)),
    ])
}

/// Update the volume item
fn update_volume(icon: &str, percentage: u8) -> Result<(), std::io::Error> {
    set_item("volume", &[
        ("icon", icon),
        ("label", &format!("{}%", percentage)),
    ])
}

/// The last weather reading, kept so a display change can repaint without
/// another network round trip.
static LAST_WEATHER: Mutex<Option<providers::WeatherInfo>> = Mutex::new(None);

/// Paint the two weather items. `weather` sits right of the notch and is
/// shown on the built-in display only; `weather.wide` is centered and shown
/// on every other display. Both show the same reading. An item with no
/// display to go on (clamshell mode, no external screen) is hidden, as are
/// both until the first reading arrives.
fn paint_weather() -> Result<(), std::io::Error> {
    let info = LAST_WEATHER.lock().map(|w| w.clone()).unwrap_or_default();
    let (builtin, external) = providers::display_arrangement_ids();

    let mut batch = SketchybarBatch::new();
    match &info {
        Some(info) => {
            let label = match info.wind {
                Some(wind) => format!("{:.1}° {:.0} m/s", info.temp, wind),
                None => format!("{:.1}°", info.temp),
            };
            for (item, displays) in [("weather", builtin), ("weather.wide", external)] {
                if displays.is_empty() {
                    batch.set(item, &[("drawing", "off")]);
                } else {
                    let display_list = displays
                        .iter()
                        .map(|d| d.to_string())
                        .collect::<Vec<_>>()
                        .join(",");
                    batch.set(item, &[
                        ("icon", &info.icon),
                        ("label", &label),
                        ("display", &display_list),
                        ("drawing", "on"),
                    ]);
                }
            }
        }
        None => {
            batch.set("weather", &[("drawing", "off")]);
            batch.set("weather.wide", &[("drawing", "off")]);
        }
    }
    batch.execute()
}

/// Label fonts of the brew item: the Lucide icon font while an upgrade runs,
/// and the text font from sketchybarrc otherwise.
const LUCIDE_LABEL_FONT: &str = "lucide:Regular:13.0";
const TEXT_LABEL_FONT: &str = "JetbrainsMono Nerd Font:Bold:13.0";

/// Update the brew outdated item
fn update_brew(icon: &str, formulae: usize, casks: usize) -> Result<(), std::io::Error> {
    let total = formulae + casks;
    let label = if total == 0 {
        "✓".to_string()
    } else {
        format!("{}", total)
    };
    set_item("brew", &[
        ("icon", icon),
        ("label", &label),
    ])
}

/// Update the Microsoft Teams notification item
fn update_teams(icon: &str, icon_color: &str, border_color: &str, notification_count: u32) -> Result<(), std::io::Error> {
    let label = if notification_count > 0 {
        format!("{}", notification_count)
    } else {
        String::new()
    };
    set_item("teams", &[
        ("icon", icon),
        ("icon.color", icon_color),
        ("background.border_color", border_color),
        ("label", &label),
        ("drawing", "on"),
    ])
}

/// Shared state for the daemon
#[derive(Debug)]
pub struct DaemonState {
    /// Last workspace change timestamp for debouncing
    pub last_workspace_change: Option<Instant>,
    /// Previously rendered workspaces (to detect which ones need clearing)
    pub previous_workspaces: HashSet<String>,
    /// Signature of the window set as of the last workspace repaint. The
    /// watcher in `window_watch` compares it to find changes that no event told us about.
    pub last_window_signature: Option<String>,
    /// Configuration
    pub config: crate::config::Config,
}

impl DaemonState {
    pub fn new(config: crate::config::Config) -> Self {
        Self {
            last_workspace_change: None,
            previous_workspaces: HashSet::new(),
            last_window_signature: None,
            config,
        }
    }
}

pub fn handle_clock_refresh() {
    let time = providers::get_clock();
    if let Err(e) = update_clock(&time) {
        eprintln!("Failed to update clock: {}", e);
    }
}

pub fn handle_battery_refresh(power_source: Option<String>, config: &crate::config::Config) {
    if let Some(info) = providers::get_battery(power_source) {
        if let Err(e) = update_battery(info.icon(), info.icon_color(config), info.label_color(config), info.percentage) {
            eprintln!("Failed to update battery: {}", e);
        }
    }
}

pub fn handle_weather_refresh() {
    // Offline or FMI down: leave the previous reading on the bar rather than
    // clearing it, matching the reference module's "keep last reading" behaviour.
    if let Some(info) = providers::get_weather() {
        if let Ok(mut last) = LAST_WEATHER.lock() {
            *last = Some(info);
        }
        if let Err(e) = paint_weather() {
            eprintln!("Failed to update weather: {}", e);
        }
    }
}

/// A display was added, removed, or the active display changed: move the
/// weather items to the displays they belong on.
pub fn handle_display_changed() {
    if let Err(e) = paint_weather() {
        eprintln!("Failed to move weather items: {}", e);
    }
}

pub fn handle_brew_refresh() {
    let info = providers::get_brew_outdated();
    if let Err(e) = update_brew(info.icon(), info.formulae, info.casks) {
        eprintln!("Failed to update brew: {}", e);
    }
}

pub fn handle_teams_refresh() {
    let info = providers::get_teams_notifications();
    if let Err(e) = update_teams(
        info.icon(),
        info.icon_color(),
        info.border_color(),
        info.notification_count,
    ) {
        eprintln!("Failed to update teams: {}", e);
    }
}

pub fn handle_teams_clicked() {
    // Create continuous pulsing animation for the teams icon
    let mut batch = SketchybarBatch::new();

    // Chain 8 bounce cycles (up and down) for ~4 seconds total
    for _ in 0..1 {
        batch.animate("sin", 15)  // Bounce up (0.25 seconds)
             .set("teams", &[("icon.y_offset", "-3")])
             .animate("sin", 15)  // Bounce down (0.25 seconds)
             .set("teams", &[("icon.y_offset", "0")]);
    }

    if let Err(e) = batch.execute() {
        eprintln!("Failed to start teams animation: {}", e);
    }

    thread::spawn(|| {
        // Open Microsoft Teams app
        let result = Command::new("open")
            .arg("/Applications/Microsoft Teams.app")
            .output();

        match result {
            Ok(output) => {
                if !output.status.success() {
                    eprintln!("Failed to open Microsoft Teams: {}", String::from_utf8_lossy(&output.stderr));
                }
            }
            Err(e) => eprintln!("Failed to run open command: {}", e),
        }

        // Wait for 2 seconds
        thread::sleep(Duration::from_secs(2));

        // Reset icon offset and refresh teams notifications
        if let Err(e) = set_item("teams", &[("icon.y_offset", "0")]) {
            eprintln!("Failed to reset teams icon offset: {}", e);
        }
        handle_teams_refresh();
    });
}

pub fn handle_system_refresh(
    prev_cpu: &mut Option<providers::CpuTicks>,
    prev_net: &mut providers::NetSample,
) {
    let (info, cur_cpu) = providers::get_system_info(*prev_cpu);
    *prev_cpu = cur_cpu;

    let cur_net = providers::read_net_sample();
    let (down, up) = providers::net_rates(prev_net, &cur_net);
    *prev_net = cur_net;

    let mut batch = SketchybarBatch::new();
    batch.set("network", &[
        ("label", &format!(
            "{}↓{}↑",
            providers::format_rate(down),
            providers::format_rate(up)
        )),
    ]);
    batch.set("cpu", &[
        ("label", &format!("{}%", info.cpu_percentage)),
    ]);
    batch.set("ram", &[
        ("label", &format!("{:.1}/{:.0}GB", info.ram_used_gb, info.ram_total_gb)),
    ]);
    if let Err(e) = batch.execute() {
        eprintln!("Failed to update cpu/ram/network: {}", e);
    }
}

pub fn handle_brew_upgrade() {
    // Show the Lucide refresh-cw glyph in the label while the upgrade runs.
    // The label font is a text font, so switch it to Lucide for the glyph and
    // restore it when the upgrade is done.
    if let Err(e) = set_item("brew", &[
        ("label.font", LUCIDE_LABEL_FONT),
        ("label", "\u{e145}"),
        ("label.y_offset", "0"),
    ]) {
        eprintln!("Failed to set brew refreshing label: {}", e);
    }

    // Create continuous pulsing animation for the label (refresh icon)
    // Since rotation is not supported, use a bouncing y_offset animation
    let mut batch = SketchybarBatch::new();

    // Chain 60 bounce cycles (up and down) for ~30 seconds total
    for _ in 0..60 {
        batch.animate("sin", 15)  // Bounce up (0.25 seconds)
             .set("brew", &[("label.y_offset", "-3")])
             .animate("sin", 15)  // Bounce down (0.25 seconds)
             .set("brew", &[("label.y_offset", "0")]);
    }

    if let Err(e) = batch.execute() {
        eprintln!("Failed to start brew animation: {}", e);
    }

    // Run brew upgrade in a separate thread so animation can continue
    thread::spawn(|| {
        let result = Command::new("brew")
            .arg("upgrade")
            .output();

        match result {
            Ok(output) => {
                if !output.status.success() {
                    eprintln!("brew upgrade failed: {}", String::from_utf8_lossy(&output.stderr));
                }
            }
            Err(e) => eprintln!("Failed to run brew upgrade: {}", e),
        }

        // Refresh the brew count after upgrade completes (this cancels animation and resets offset)
        if let Err(e) = set_item("brew", &[("label.y_offset", "0"), ("label.font", TEXT_LABEL_FONT)]) {
            eprintln!("Failed to reset brew offset: {}", e);
        }
        handle_brew_refresh();
    });
}

pub fn handle_volume_refresh(vol: Option<u8>) {
    let info = if let Some(v) = vol {
        providers::VolumeInfo { percentage: v, muted: v == 0 }
    } else if let Some(v) = providers::get_volume() {
        v
    } else {
        return;
    };

    if let Err(e) = update_volume(info.icon(), info.percentage) {
        eprintln!("Failed to update volume: {}", e);
    }
}

/// The window manager that the config selects.
fn window_manager(state: &Arc<Mutex<DaemonState>>) -> WindowManager {
    state.lock().map(|s| s.config.window_manager).unwrap_or_default()
}

/// Dots for a workspace item: one filled dot for each window, or one outlined
/// dot when the workspace has no windows. An en space (U+2002) separates the
/// dots. It is narrower than a normal space in the monospace bar font.
fn workspace_dots(window_count: usize) -> String {
    if window_count == 0 {
        "\u{25cb}".to_string()
    } else {
        vec!["\u{25cf}"; window_count].join("\u{2002}")
    }
}

/// Repaint every workspace item. Returns whether a repaint actually happened:
/// `false` means the call was debounced or the state lock was poisoned, so the
/// bar may still be showing something older than what the window manager reports.
pub fn handle_workspace_refresh(state: &Arc<Mutex<DaemonState>>) -> bool {
    // Debounce: Check if enough time has passed since the last workspace change
    let now = Instant::now();
    let should_process = if let Ok(mut s) = state.lock() {
        if let Some(last_change) = s.last_workspace_change {
            if now.duration_since(last_change) < Duration::from_millis(100) {
                false // Debounce - skip this event
            } else {
                s.last_workspace_change = Some(now);
                true
            }
        } else {
            s.last_workspace_change = Some(now);
            true
        }
    } else {
        return false;
    };

    if !should_process {
        return false; // Event was debounced
    }

    let wm = window_manager(state);

    // Small delay to let the window manager settle its internal state
    thread::sleep(Duration::from_millis(10));

    let mut infos = wm::workspace_infos(wm);

    // Manual display mapping: swap display 2 with display 3. AeroSpace numbers
    // displays in NSScreen order, which is not sketchybar's order. Rift display
    // IDs come from sketchybar's own order, so they need no swap.
    if wm == WindowManager::Aerospace {
        for info in infos.values_mut() {
            if info.display_id == 2 {
                info.display_id = 3;
            } else if info.display_id == 3 {
                info.display_id = 2;
            }
        }
    }

    // All displays that have a workspace, to clear old items on each of them.
    let all_displays: HashSet<u32> = infos.values().map(|info| info.display_id).collect();

    // Get the set of current workspaces
    let current_workspaces: HashSet<String> = infos.keys().cloned().collect();

    // Get previous workspaces, config, and update state
    let (previous_workspaces, config) = if let Ok(mut s) = state.lock() {
        let prev = s.previous_workspaces.clone();
        let cfg = s.config.clone();
        s.previous_workspaces = current_workspaces.clone();
        (prev, cfg)
    } else {
        return false;
    };

    // Generate gradient colors from border_active_color (10 steps)
    let gradient_colors = get_workspace_gradient_colors(&config);

    // Find workspaces that need to be cleared
    let workspaces_to_clear: HashSet<String> = previous_workspaces
        .difference(&current_workspaces)
        .cloned()
        .collect();

    // Create a batch per display
    let mut batches: HashMap<u32, SketchybarBatch> = HashMap::new();

    // Clear workspaces that are no longer in the window manager's list
    for ws_id in workspaces_to_clear {
        let item_name = format!("workspace.{}", ws_id);
        for display_id in &all_displays {
            let batch = batches.entry(*display_id).or_insert_with(SketchybarBatch::new);
            batch.set(&item_name, &[
                ("drawing", "off"),
                ("display", &display_id.to_string()),
            ]);
        }
    }

    // Sort workspaces by ID to get consistent bar order
    let mut sorted_ws_ids: Vec<&String> = infos.keys().collect();
    sorted_ws_ids.sort();

    // Process each workspace in sorted order (matching bar position)
    for (position, ws_id) in sorted_ws_ids.iter().enumerate() {
        let info = &infos[*ws_id];
        let is_focused = info.is_focused;
        let display_id = info.display_id;
        let item_name = format!("workspace.{}", ws_id);
        let display_str = display_id.to_string();
        let batch = batches.entry(display_id).or_insert_with(SketchybarBatch::new);

        // The item icon is the workspace ID, and the item label is the window
        // dots, so the dots come after the ID. The "label" colors in the config
        // apply to the ID, and the "icon" colors apply to the dots.
        let id_color = if is_focused {
            &config.workspace_focused_label_color
        } else {
            &config.workspace_unfocused_label_color
        };
        let dot_color = if is_focused {
            &config.workspace_focused_icon_color
        } else {
            &config.workspace_unfocused_icon_color
        };
        let background_drawing = if is_focused { "on" } else { "off" };

        let mut settings = vec![
            ("icon", ws_id.to_string()),
            ("icon.color", id_color.to_string()),
            ("label", workspace_dots(info.apps.len())),
            ("label.color", dot_color.to_string()),
            ("drawing", "on".to_string()),
            ("background.drawing", background_drawing.to_string()),
            ("display", display_str.clone()),
        ];

        if is_focused {
            // Render the focused workspace as a thin bottom underline rather
            // than a full background fill: a short background strip pushed to
            // the bottom of the item. The square corners come from the item's
            // static background.corner_radius=0 in sketchybarrc.
            let ws_index = position % gradient_colors.len();
            let bg_color = &gradient_colors[ws_index];
            settings.push(("background.color", bg_color.to_string()));
            settings.push(("background.height", "2".to_string()));
            settings.push(("background.y_offset", "-15".to_string()));
        }

        let settings_refs: Vec<(&str, &str)> = settings
            .iter()
            .map(|(k, v)| (*k, v.as_str()))
            .collect();

        batch.set(&item_name, &settings_refs);
    }

    // Execute all batches
    for (display_id, batch) in batches {
        if let Err(e) = batch.execute() {
            eprintln!("Failed to update workspaces on display {}: {}", display_id, e);
        }
    }

    // Update borders active color
    std::thread::sleep(std::time::Duration::from_millis(40));
    let border_arg = format!("active_color={}", config.border_active_color);
    if let Err(e) = Command::new("/opt/homebrew/bin/borders")
        .arg(&border_arg)
        .status()
    {
        eprintln!("Failed to update borders color: {}", e);
    }

    true
}

/// Focus a workspace and launch its configured app if not already running.
/// Runs in a background thread so the daemon doesn't block on the app launch.
pub fn handle_aerospace_focus(workspace: String, state: &Arc<Mutex<DaemonState>>) {
    let state = Arc::clone(state);
    thread::spawn(move || {
        aerospace_focus::focus_workspace(window_manager(&state), &workspace);
        handle_workspace_refresh(&state);
    });
}

/// Focus a workspace (from a click on its bar item). The window manager's
/// workspace event repaints the bar.
pub fn handle_focus_workspace(workspace: String, state: &Arc<Mutex<DaemonState>>) {
    let wm = window_manager(state);
    thread::spawn(move || {
        wm::focus_workspace(wm, &workspace);
    });
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_dots() {
        assert_eq!(workspace_dots(0), "\u{25cb}");
        assert_eq!(workspace_dots(1), "\u{25cf}");
        assert_eq!(workspace_dots(3), "\u{25cf}\u{2002}\u{25cf}\u{2002}\u{25cf}");
    }
}
