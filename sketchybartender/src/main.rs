mod aerospace;
mod config;
mod daemon;
mod handlers;
mod icon_map;
mod mach_client;
mod providers;

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use chrono::{Local, Timelike};
use handlers::DaemonState;

fn main() {
    // Load configuration
    let config = config::Config::load();

    // Shared state
    let state = Arc::new(Mutex::new(DaemonState::new(config.clone())));

    // Spawn brew refresh early (before delay) since it takes the longest
    let brew_interval = config.brew_interval;
    thread::spawn(move || {
        // Initial refresh
        handlers::handle_brew_refresh();
        
        loop {
            thread::sleep(Duration::from_secs(brew_interval));
            handlers::handle_brew_refresh();
        }
    });

    // Wait for sketchybar to be ready
    thread::sleep(Duration::from_millis(200));

    // Spawn refresh thread for workspace (event-driven, but needs initial refresh)
    let workspace_state = Arc::clone(&state);
    thread::spawn(move || {
        // Initial refresh
        handlers::handle_workspace_refresh(&workspace_state);
    });

    // Spawn timer threads for periodic updates using configured intervals
    // Smart clock refresh: poll every 2 seconds until minute changes, then switch to 1-minute intervals
    thread::spawn(move || {
        // Initial refresh
        handlers::handle_clock_refresh();
        
        // Track the current minute
        let last_minute = Local::now().minute();
        
        // Poll every 2 seconds until we detect a minute change
        loop {
            thread::sleep(Duration::from_secs(2));
            let current_minute = Local::now().minute();
            
            if current_minute != last_minute {
                // Minute changed! Wait 0.5 seconds and switch to 1-minute intervals
                thread::sleep(Duration::from_millis(500));
                handlers::handle_clock_refresh();
                break;
            }
            
            handlers::handle_clock_refresh();
        }
        
        // Now use 1-minute intervals, synchronized to the minute boundary
        loop {
            thread::sleep(Duration::from_secs(60));
            handlers::handle_clock_refresh();
        }
    });

    let battery_interval = config.battery_interval;
    thread::spawn(move || {
        // Initial refresh
        handlers::handle_battery_refresh(None);
        
        loop {
            thread::sleep(Duration::from_secs(battery_interval));
            handlers::handle_battery_refresh(None);
        }
    });

    let teams_interval = config.teams_interval;
    thread::spawn(move || {
        // Initial refresh
        handlers::handle_teams_refresh();
        
        loop {
            thread::sleep(Duration::from_secs(teams_interval));
            handlers::handle_teams_refresh();
        }
    });

    let system_interval = config.system_interval;
    thread::spawn(move || {
        // Initial refresh
        handlers::handle_system_refresh();
        
        loop {
            thread::sleep(Duration::from_secs(system_interval));
            handlers::handle_system_refresh();
        }
    });

    // Start the daemon socket listener
    daemon::start_daemon(state);
}
