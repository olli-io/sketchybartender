use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::aerospace;
use crate::config::Config;
use crate::icon_map;
use crate::providers;

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
            self.args.push(format!("{}={}", key, value));
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

        let status = Command::new("sketchybar")
            .args(&self.args)
            .status()?;

        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "sketchybar command failed",
            ))
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
    let app_name = app.unwrap_or_else(|| "unknown".to_string());
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
    // This helps avoid race conditions when aerospace is still updating
    thread::sleep(Duration::from_millis(10));

    // Get all unique display IDs to determine if we're on single or multi-monitor setup
    let all_displays: HashSet<u32> = {
        let temp_infos = aerospace::get_workspace_infos(false);
        temp_infos.values().map(|info| info.display_id).collect()
    };
    let is_single_monitor = all_displays.len() == 1;

    // Show all windows on multiple monitors, one icon per app on single monitor
    // This queries aerospace fresh each time - no caching of workspace state
    let infos = aerospace::get_workspace_infos(!is_single_monitor);

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

    // Find workspaces that need to be cleared (were rendered before but not in current list)
    let workspaces_to_clear: HashSet<String> = previous_workspaces
        .difference(&current_workspaces)
        .cloned()
        .collect();

    // Create a batch per display
    let mut batches: HashMap<u32, SketchybarBatch> = HashMap::new();

    // Clear workspaces that are no longer in aerospace's list
    for ws_id in workspaces_to_clear {
        let item_name = format!("workspace.{}", ws_id);

        // Clear on all displays
        for display_id in &all_displays {
            let batch = batches.entry(*display_id).or_insert_with(SketchybarBatch::new);
            batch.set(&item_name, &[
                ("drawing", "off"),
                ("background.drawing", "off"),
                ("icon.drawing", "off"),
                ("icon", ""),
                ("display", &display_id.to_string()),
            ]);
        }
    }

    // Process each workspace from the fresh aerospace data
    // We only use infos.keys() which represents the current live state from aerospace
    for (ws_id, info) in &infos {
        let has_apps = !info.apps.is_empty();
        let is_focused = info.is_focused;
        let icons = info.icons.as_str();
        let display_id = info.display_id;

        let item_name = format!("workspace.{}", ws_id);

        // The display_id from aerospace directly corresponds to Sketchybar's display ID
        let batch = batches.entry(display_id).or_insert_with(SketchybarBatch::new);

        if has_apps && is_focused {
            batch.set(&item_name, &[
                ("label", &format!("[{}]", ws_id)),
                ("label.color", &config.workspace_focused_label_color),
                ("icon", icons),
                ("icon.color", &config.workspace_focused_icon_color),
                ("icon.drawing", "on"),
                ("drawing", "on"),
                ("background.drawing", "on"),
                ("background.color", &config.workspace_bg_color),
                ("display", &display_id.to_string()),
            ]);
        } else if has_apps {
            batch.set(&item_name, &[
                ("label", &format!("[{}]", ws_id)),
                ("label.color", &config.workspace_unfocused_label_color),
                ("icon.color", &config.workspace_unfocused_icon_color),
                ("icon", icons),
                ("icon.drawing", "on"),
                ("drawing", "on"),
                ("background.drawing", "off"),
                ("display", &display_id.to_string()),
            ]);
        } else if is_focused {
            batch.set(&item_name, &[
                ("label", &format!("\u{f444} [{}]", ws_id)),
                ("label.color", &config.workspace_focused_label_color),
                ("icon.color", &config.workspace_focused_icon_color),
                ("icon", ""),
                ("drawing", "on"),
                ("icon.drawing", "off"),
                ("background.drawing", "on"),
                ("background.color", &config.workspace_bg_color),
                ("display", &display_id.to_string()),
            ]);
        } else {
            // Empty and not focused
            if is_single_monitor {
                // Hide completely when single monitor
                batch.set(&item_name, &[
                    ("drawing", "off"),
                    ("background.drawing", "off"),
                    ("icon.drawing", "off"),
                    ("display", &display_id.to_string()),
                ]);
            } else {
                // Show when multiple monitors
                batch.set(&item_name, &[
                    ("label", &format!("\u{f444} [{}]", ws_id)),
                    ("label.color", &config.workspace_unfocused_label_color),
                    ("icon.color", &config.workspace_unfocused_icon_color),
                    ("icon", ""),
                    ("drawing", "on"),
                    ("icon.drawing", "off"),
                    ("background.drawing", "off"),
                    ("display", &display_id.to_string()),
                ]);
            }
        }
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

pub fn handle_config_reload(state: &Arc<Mutex<DaemonState>>) {
    // Load the new configuration
    let new_config = Config::load();

    // Update the state with the new config
    if let Ok(mut s) = state.lock() {
        s.config = new_config;
        println!("Configuration reloaded successfully");
    } else {
        eprintln!("Failed to acquire lock for config reload");
        return;
    }

    // Trigger a workspace refresh to apply new colors
    handle_workspace_refresh(state);
}