mod aerospace;
mod config;
mod icon_map;
mod monitor_map;
mod providers;
mod sketchybar;

use std::env;
use std::io::{BufRead, BufReader};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::fs;

use monitor_map::MonitorMapper;
use sketchybar::SketchybarBatch;

/// Shared state for the daemon
#[derive(Debug)]
struct DaemonState {
    /// Current front app (for deduplication)
    front_app: String,
    /// Monitor mapper for workspace filtering
    monitor_mapper: MonitorMapper,
}

impl Default for DaemonState {
    fn default() -> Self {
        Self {
            front_app: String::new(),
            monitor_mapper: MonitorMapper::new(),
        }
    }
}

fn handle_client(stream: UnixStream, state: Arc<Mutex<DaemonState>>) {
    let reader = BufReader::new(stream);

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        let parts: Vec<&str> = line.trim().splitn(3, ' ').collect();
        match parts.get(0).map(|s| *s) {
            Some("on-volume-changed") => {
                let vol = parts.get(1).and_then(|s| s.parse().ok());
                handle_volume_refresh(vol);
            }
            Some("on-focus-changed") => handle_focus_refresh(None, &state),
            Some("on-workspace-changed") => handle_workspace_refresh(&state),
            Some("on-brew-clicked") => handle_brew_upgrade(),
            Some("trigger-teams-refresh") => handle_teams_refresh(),
            Some("on-display-configuration-changed") => handle_workspace_refresh(&state),
            Some("on-power-source-changed") => handle_battery_refresh(),
            Some("on-system-wake") => {
                handle_workspace_refresh(&state);
                handle_battery_refresh();
                handle_clock_refresh();
                handle_teams_refresh();
            }
            _ => {
                eprintln!("Unknown message: {}", line);
            }
        }
    }
}

fn handle_clock_refresh() {
    let time = providers::get_clock();
    if let Err(e) = sketchybar::update_clock(&time) {
        eprintln!("Failed to update clock: {}", e);
    }
}

fn handle_battery_refresh() {
    if let Some(info) = providers::get_battery() {
        if let Err(e) = sketchybar::update_battery(info.icon(), info.percentage) {
            eprintln!("Failed to update battery: {}", e);
        }
    }
}

fn handle_brew_refresh() {
    let info = providers::get_brew_outdated();
    if let Err(e) = sketchybar::update_brew(info.icon(), info.formulae, info.casks) {
        eprintln!("Failed to update brew: {}", e);
    }
}

fn handle_teams_refresh() {
    let info = providers::get_teams_notifications();
    if let Err(e) = sketchybar::update_teams(
        info.icon(),
        info.icon_color(),
        info.border_color(),
        info.notification_count,
    ) {
        eprintln!("Failed to update teams: {}", e);
    }
}

fn handle_brew_upgrade() {
    use std::process::Command;

    // Set the refresh icon
    if let Err(e) = sketchybar::set_item("brew", &[
        ("label", "\u{f409}"),
        ("label.y_offset", "0"),
    ]) {
        eprintln!("Failed to set brew refreshing label: {}", e);
    }

    // Create continuous pulsing animation for the label (refresh icon)
    // Since rotation is not supported, use a bouncing y_offset animation
    let mut batch = sketchybar::SketchybarBatch::new();

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
        if let Err(e) = sketchybar::set_item("brew", &[("label.y_offset", "0")]) {
            eprintln!("Failed to reset brew offset: {}", e);
        }
        handle_brew_refresh();
    });
}

fn handle_volume_refresh(vol: Option<u8>) {
    let info = if let Some(v) = vol {
        providers::VolumeInfo { percentage: v, muted: v == 0 }
    } else if let Some(v) = providers::get_volume() {
        v
    } else {
        return;
    };

    if let Err(e) = sketchybar::update_volume(info.icon(), info.percentage) {
        eprintln!("Failed to update volume: {}", e);
    }
}

fn handle_focus_refresh(app: Option<String>, state: &Arc<Mutex<DaemonState>>) {
    let app = app.or_else(|| aerospace::get_focused_app());
    
    if let Some(app_name) = &app {
        let icon = icon_map::get_icon(app_name);

        // Update state
        if let Ok(mut s) = state.lock() {
            if s.front_app == *app_name {
                return; // No change
            }
            s.front_app = app_name.clone();
        }
        
        if let Err(e) = sketchybar::update_front_app(icon, app_name) {
            eprintln!("Failed to update front_app: {}", e);
        }
    }
}

fn handle_workspace_refresh(state: &Arc<Mutex<DaemonState>>) {
    // Small delay to let aerospace settle its internal state
    // This helps avoid race conditions when aerospace is still updating
    thread::sleep(Duration::from_millis(10));

    let monitor_mappings = if let Ok(s) = state.lock() {
        s.monitor_mapper.get_mappings()
    } else {
        return;
    };

    // Check if there's only one monitor (native laptop display)
    let is_single_monitor = monitor_mappings.len() == 1;

    let infos = aerospace::get_workspace_infos();

    // Create a batch per display
    let mut batches: std::collections::HashMap<u32, SketchybarBatch> = std::collections::HashMap::new();

    for i in 1..=9 {
        let ws_id = i.to_string();
        let info = infos.get(&ws_id);
        let has_apps = info.map(|i| !i.apps.is_empty()).unwrap_or(false);
        let is_focused = info.map(|i| i.is_focused).unwrap_or(false);
        let icons = info.map(|i| i.icons.as_str()).unwrap_or("");
        let workspace_monitor = info.map(|i| i.monitor_id).unwrap_or(1);

        let item_name = format!("workspace.{}", ws_id);

        // Rotate background colors: 1,4,7 → blue, 2,5,8 → pink, 3,6,9 → green
        let bg_color = match i % 3 {
            1 => "0xff83a598", // blue
            2 => "0xffd3869b", // pink
            0 => "0xff8ec07c", // green
            _ => unreachable!(),
        };

        // Find the Sketchybar display ID for this workspace's monitor
        // We need to iterate through monitor_mappings to find the display that maps to this aerospace monitor
        let mut found = false;
        for (display_id, aerospace_monitor_id) in &monitor_mappings {
            if *aerospace_monitor_id == workspace_monitor {
                let batch = batches.entry(*display_id).or_insert_with(SketchybarBatch::new);

                if has_apps && is_focused {
                    batch.set(&item_name, &[
                        ("label", &ws_id),
                        ("label.color", "0xff1d2021"),
                        ("icon", icons),
                        ("icon.color", "0xff1d2021"),
                        ("icon.drawing", "on"),
                        ("drawing", "on"),
                        ("background.drawing", "on"),
                        ("background.color", bg_color),
                        ("display", &display_id.to_string()),
                    ]);
                } else if has_apps {
                    batch.set(&item_name, &[
                        ("label", &ws_id),
                        ("label.color", "0xffffffff"),
                        ("icon.color", "0xffffffff"),
                        ("icon", icons),
                        ("icon.drawing", "on"),
                        ("drawing", "on"),
                        ("background.drawing", "off"),
                        ("display", &display_id.to_string()),
                    ]);
                } else if is_focused {
                    batch.set(&item_name, &[
                        ("label", &format!("\u{f444} {}", ws_id)),
                        ("label.color", "0xff1d2021"),
                        ("icon.color", "0xff1d2021"),
                        ("drawing", "on"),
                        ("icon.drawing", "off"),
                        ("background.drawing", "on"),
                        ("background.color", bg_color),
                        ("display", &display_id.to_string()),
                    ]);
                } else {
                    // Empty and not focused
                    if is_single_monitor {
                        // Hide completely when single monitor
                        batch.set(&item_name, &[
                            ("drawing", "off"),
                            ("display", &display_id.to_string()),
                        ]);
                    } else {
                        // Show when multiple monitors
                        batch.set(&item_name, &[
                            ("label", &format!("\u{f444} {}", ws_id)),
                            ("label.color", "0xffffffff"),
                            ("icon.color", "0xffffffff"),
                            ("drawing", "on"),
                            ("icon.drawing", "off"),
                            ("background.drawing", "off"),
                            ("display", &display_id.to_string()),
                        ]);
                    }
                }
                found = true;
                break; // Only update on the correct display
            }
        }

        if !found {
            
            // Fallback: try to update on display 1 if no mapping found
            // This ensures the workspace is at least visible somewhere
            let batch = batches.entry(1).or_insert_with(SketchybarBatch::new);
            
            if has_apps && is_focused {
                batch.set(&item_name, &[
                    ("label", &ws_id),
                    ("label.color", "0xff1d2021"),
                    ("icon", icons),
                    ("icon.color", "0xff1d2021"),
                    ("icon.drawing", "on"),
                    ("drawing", "on"),
                    ("background.drawing", "on"),
                    ("background.color", bg_color),
                    ("display", "1"),
                ]);
            } else if has_apps {
                batch.set(&item_name, &[
                    ("label", &ws_id),
                    ("label.color", "0xffffffff"),
                    ("icon.color", "0xffffffff"),
                    ("icon", icons),
                    ("icon.drawing", "on"),
                    ("drawing", "on"),
                    ("background.drawing", "off"),
                    ("display", "1"),
                ]);
            } else if is_focused {
                batch.set(&item_name, &[
                    ("label", &format!("\u{f444} {}", ws_id)),
                    ("label.color", "0xff1d2021"),
                    ("icon.color", "0xff1d2021"),
                    ("drawing", "on"),
                    ("icon.drawing", "off"),
                    ("background.drawing", "on"),
                    ("background.color", bg_color),
                    ("display", "1"),
                ]);
            } else {
                // Empty and not focused
                if is_single_monitor {
                    // Hide completely when single monitor
                    batch.set(&item_name, &[
                        ("drawing", "off"),
                        ("display", "1"),
                    ]);
                } else {
                    // Show when multiple monitors
                    batch.set(&item_name, &[
                        ("label", &format!("\u{f444} {}", ws_id)),
                        ("label.color", "0xffffffff"),
                        ("icon.color", "0xffffffff"),
                        ("drawing", "on"),
                        ("icon.drawing", "off"),
                        ("background.drawing", "off"),
                        ("display", "1"),
                    ]);
                }
            }
        }
    }

    // Execute all batches
    for (display_id, batch) in batches {
        if let Err(e) = batch.execute() {
            eprintln!("Failed to update workspaces on display {}: {}", display_id, e);
        }
    }
}

fn get_socket_path() -> PathBuf {
    let cache_dir = env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = env::var("HOME").expect("HOME not set");
            PathBuf::from(home).join(".cache")
        });

    cache_dir.join("sketchybar").join("helper.sock")
}

fn main() {
    // Load configuration
    let config = config::Config::load();

    let socket_path = get_socket_path();

    // Ensure parent directory exists
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent).expect("Failed to create cache directory");
    }

    // Remove existing socket
    let _ = fs::remove_file(&socket_path);

    // Create listener
    let listener = UnixListener::bind(&socket_path).expect("Failed to bind socket");
    println!("Sketchybar helper daemon listening on {:?}", socket_path);

    // Shared state
    let state = Arc::new(Mutex::new(DaemonState::default()));

    // Initial refresh
    handle_workspace_refresh(&state);
    handle_clock_refresh();
    handle_battery_refresh();
    handle_focus_refresh(None, &state);
    handle_brew_refresh();
    handle_teams_refresh();

    // Spawn timer threads for periodic updates using configured intervals
    let clock_interval = config.clock_interval;
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(clock_interval));
            handle_clock_refresh();
        }
    });

    let battery_interval = config.battery_interval;
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(battery_interval));
            handle_battery_refresh();
        }
    });

    let brew_interval = config.brew_interval;
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(brew_interval));
            handle_brew_refresh();
        }
    });

    let teams_interval = config.teams_interval;
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(teams_interval));
            handle_teams_refresh();
        }
    });

    // Accept connections
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    handle_client(stream, state);
                });
            }
            Err(e) => {
                eprintln!("Connection error: {}", e);
            }
        }
    }
}
