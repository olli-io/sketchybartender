use std::env;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use crate::handlers::{
    DaemonState,
    handle_battery_refresh,
    handle_brew_upgrade,
    handle_config_reload,
    handle_focus_refresh,
    handle_teams_refresh,
    handle_volume_refresh,
    handle_workspace_refresh,
};

pub fn handle_client(stream: UnixStream, state: Arc<Mutex<DaemonState>>) {
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
            Some("on-focus-changed") => {
                let app_name = parts.get(1).map(|s| s.to_string());
                handle_focus_refresh(app_name, &state);
            }
            Some("on-workspace-changed") => handle_workspace_refresh(&state),
            Some("on-brew-clicked") => handle_brew_upgrade(),
            Some("trigger-teams-refresh") => handle_teams_refresh(),
            Some("on-display-configuration-changed") => handle_workspace_refresh(&state),
            Some("on-power-source-changed") => {
                let power_source = parts.get(1).map(|s| s.to_string());
                handle_battery_refresh(power_source);
            }
            Some("on-system-wake") => {
                handle_workspace_refresh(&state);
                handle_battery_refresh(None);
                crate::handlers::handle_clock_refresh();
                handle_teams_refresh();
            }
            Some("reload-config") => {
                crate::handlers::handle_config_reload(&state);
            }
            _ => {
                eprintln!("Unknown message: {}", line);
            }
        }
    }
}

pub fn get_socket_path() -> PathBuf {
    let cache_dir = env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = env::var("HOME").expect("HOME not set");
            PathBuf::from(home).join(".cache")
        });

    cache_dir.join("sketchybar").join("helper.sock")
}

pub fn start_daemon(state: Arc<Mutex<DaemonState>>) {
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

    // Small delay to ensure sketchybar has initialized and is ready to receive updates
    thread::sleep(std::time::Duration::from_millis(50));

    // Perform initial refresh after sketchybar is ready
    handle_workspace_refresh(&state);
    handle_battery_refresh(None);
    crate::handlers::handle_clock_refresh();
    handle_teams_refresh();

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
