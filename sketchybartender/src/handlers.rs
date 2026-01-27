use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::aerospace;
use crate::icon_map;
use crate::mach_client;
use crate::providers;

/// Parse a color like "0xffbb60cd" to (r, g, b) tuple
fn parse_color(color: &str) -> Option<(u8, u8, u8)> {
    let hex = color.strip_prefix("0x")?;
    // Handle both 8-char (with alpha) and potentially malformed inputs
    if hex.len() < 6 {
        return None;
    }
    // Skip alpha (first 2 chars if 8 chars), parse RGB from last 6 chars
    let rgb_start = if hex.len() >= 8 { hex.len() - 6 } else { 0 };
    let r = u8::from_str_radix(&hex[rgb_start..rgb_start + 2], 16).ok()?;
    let g = u8::from_str_radix(&hex[rgb_start + 2..rgb_start + 4], 16).ok()?;
    let b = u8::from_str_radix(&hex[rgb_start + 4..rgb_start + 6], 16).ok()?;
    Some((r, g, b))
}

/// Parse gradient string like "gradient(top_left=0xffbb60cd,bottom_right=0xfffabd2f)"
fn parse_gradient(gradient: &str) -> Option<((u8, u8, u8), (u8, u8, u8))> {
    let inner = gradient.strip_prefix("gradient(")?.strip_suffix(")")?;
    let mut top_left = None;
    let mut bottom_right = None;

    for part in inner.split(',') {
        if let Some((key, value)) = part.split_once('=') {
            match key.trim() {
                "top_left" => top_left = parse_color(value.trim()),
                "bottom_right" => bottom_right = parse_color(value.trim()),
                _ => {}
            }
        }
    }

    Some((top_left?, bottom_right?))
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

/// Get gradient colors from border_active_color config (15 steps)
fn get_workspace_gradient_colors(config: &crate::config::Config) -> Vec<String> {
    if let Some((start, end)) = parse_gradient(&config.border_active_color) {
        generate_gradient(start, end, 15)
    } else {
        // Fallback: use workspace_bg_color for all
        vec![config.workspace_bg_color.clone(); 15]
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

/// Update the front app item
fn update_front_app(icon: &str, app_name: &str) -> Result<(), std::io::Error> {
    set_item("front_app", &[
        ("icon", icon),
        ("label", &format!("❯ {}", app_name)),
    ])
}

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
    /// Current front app (for deduplication)
    pub front_app: String,
    /// Last workspace change timestamp for debouncing
    pub last_workspace_change: Option<Instant>,
    /// Previously rendered workspaces (to detect which ones need clearing)
    pub previous_workspaces: HashSet<String>,
    /// Configuration
    pub config: crate::config::Config,
}

impl DaemonState {
    pub fn new(config: crate::config::Config) -> Self {
        Self {
            front_app: String::new(),
            last_workspace_change: None,
            previous_workspaces: HashSet::new(),
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

pub fn handle_battery_refresh(power_source: Option<String>) {
    if let Some(info) = providers::get_battery(power_source) {
        if let Err(e) = update_battery(info.icon(), info.icon_color(), info.label_color(), info.percentage) {
            eprintln!("Failed to update battery: {}", e);
        }
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

pub fn handle_system_refresh() {
    let info = providers::get_system_info();
    if let Err(e) = set_item("sysinfo", &[
        ("label", &format!("{:.1}/{:.0}GB", info.ram_used_gb, info.ram_total_gb)),
    ]) {
        eprintln!("Failed to update sysinfo: {}", e);
    }
}

pub fn handle_brew_upgrade() {
    // Set the refresh icon
    if let Err(e) = set_item("brew", &[
        ("label", "\u{f409}"),
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
        if let Err(e) = set_item("brew", &[("label.y_offset", "0")]) {
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

pub fn handle_focus_refresh(app: Option<String>, state: &Arc<Mutex<DaemonState>>) {
    // Get app name from parameter or query aerospace
    let mut app_name = match app {
        Some(name) => name,
        None => {
            // Fallback: query aerospace for the focused window
            Command::new("aerospace")
                .args(["list-windows", "--focused", "--format", "%{app-name}"])
                .output()
                .ok()
                .and_then(|output| {
                    if output.status.success() {
                        String::from_utf8(output.stdout).ok()
                    } else {
                        None
                    }
                })
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        }
    };
    
    // If app_name is empty, don't update (no focused window)
    if app_name.is_empty() {
        return;
    }
    
    // Remove "Microsoft " prefix from app names
    if app_name.starts_with("Microsoft ") {
        app_name = app_name.strip_prefix("Microsoft ").unwrap().to_string();
    }
    
    let icon = icon_map::get_icon(&app_name);

    // Update state
    if let Ok(mut s) = state.lock() {
        if s.front_app == app_name {
            return; // No change
        }
        s.front_app = app_name.clone();
    }

    if let Err(e) = update_front_app(icon, &app_name) {
        eprintln!("Failed to update front_app: {}", e);
    }
}

/// Helper to build workspace label with bracket formatting
fn format_workspace_label(ws_id: &str, has_icon: bool) -> String {
    if has_icon {
        format!("[{}]", ws_id)
    } else {
        format!("\u{f444} [{}]", ws_id)
    }
}

pub fn handle_workspace_refresh(state: &Arc<Mutex<DaemonState>>) {
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
        return;
    };

    if !should_process {
        return; // Event was debounced
    }

    // Small delay to let aerospace settle its internal state
    thread::sleep(Duration::from_millis(10));

    // Get all unique display IDs to determine if we're on single or multi-monitor setup
    let all_displays: HashSet<u32> = {
        let temp_infos = aerospace::get_workspace_infos(false);
        temp_infos.values().map(|info| info.display_id).collect()
    };
    let is_single_monitor = all_displays.len() == 1;

    // Show all windows on multiple monitors, one icon per app on single monitor
    let mut infos = aerospace::get_workspace_infos(!is_single_monitor);
    
    // Manual display mapping: swap display 2 with display 3
    for info in infos.values_mut() {
        if info.display_id == 2 {
            info.display_id = 3;
        } else if info.display_id == 3 {
            info.display_id = 2;
        }
    }

    // Get the set of current workspaces
    let current_workspaces: HashSet<String> = infos.keys().cloned().collect();

    // Get previous workspaces, config, and update state
    let (previous_workspaces, config) = if let Ok(mut s) = state.lock() {
        let prev = s.previous_workspaces.clone();
        let cfg = s.config.clone();
        s.previous_workspaces = current_workspaces.clone();
        (prev, cfg)
    } else {
        return;
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

    // Clear workspaces that are no longer in aerospace's list
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
        let has_apps = !info.apps.is_empty();
        let is_focused = info.is_focused;
        let display_id = info.display_id;
        let item_name = format!("workspace.{}", ws_id);
        let display_str = display_id.to_string();
        let batch = batches.entry(display_id).or_insert_with(SketchybarBatch::new);

        // Determine colors and states
        let label_color = if is_focused {
            &config.workspace_focused_label_color
        } else {
            &config.workspace_unfocused_label_color
        };
        let icon_color = if is_focused {
            &config.workspace_focused_icon_color
        } else {
            &config.workspace_unfocused_icon_color
        };
        let icon_value = if has_apps { &info.icons } else { "" };
        let icon_drawing = if has_apps { "on" } else { "off" };
        let background_drawing = if is_focused { "on" } else { "off" };

        let mut settings = vec![
            ("label", format_workspace_label(ws_id, has_apps)),
            ("label.color", label_color.to_string()),
            ("icon", icon_value.to_string()),
            ("icon.color", icon_color.to_string()),
            ("icon.drawing", icon_drawing.to_string()),
            ("drawing", "on".to_string()),
            ("background.drawing", background_drawing.to_string()),
            ("display", display_str.clone()),
        ];

        if is_focused {
            // Use gradient color based on position in bar (0-indexed)
            let ws_index = position % gradient_colors.len();
            let bg_color = &gradient_colors[ws_index];
            settings.push(("background.color", bg_color.to_string()));
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
}