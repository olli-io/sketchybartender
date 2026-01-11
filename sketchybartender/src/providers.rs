use std::process::Command;
use sysinfo::System;
use chrono::Local;

/// Battery information
#[derive(Debug, Clone)]
pub struct BatteryInfo {
    pub percentage: u8,
    pub is_charging: bool,
}

impl BatteryInfo {
    /// Get the appropriate icon for the battery state
    pub fn icon(&self) -> &'static str {
        match self.percentage {
            90..=100 => "\u{f240}",
            70..=89 => "\u{f241}",
            40..=69 => "\u{f242}",
            10..=39 => "\u{f243}",
            _ => "\u{f244}",
        }
    }

    /// Get the icon color based on charging state
    pub fn icon_color(&self) -> &'static str {
        if self.is_charging {
            "0xfffabd2f" // Yellow when charging
        } else if self.percentage <= 10 {
            "0xfffb4934" // Red when battery is critically low
        } else {
            "0xffffffff" // White when not charging
        }
    }

    pub fn label_color(&self) -> &'static str {
        self.icon_color()
    }
}

/// Get current battery information
/// If power_source is provided (from sketchybar event), use it directly instead of querying pmset
pub fn get_battery(power_source: Option<String>) -> Option<BatteryInfo> {
    let output = Command::new("pmset")
        .args(["-g", "batt"])
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse percentage - look for word containing '%' (e.g., "26%;" or "100%")
    let percentage = stdout
        .split_whitespace()
        .find(|s| s.contains('%'))
        .and_then(|s| {
            // Extract digits before the '%' sign
            s.split('%').next()?.parse::<u8>().ok()
        })?;

    // Check if charging - use provided power_source if available, otherwise parse from pmset output
    let is_charging = if let Some(source) = power_source {
        source == "AC"
    } else {
        stdout.contains("AC Power")
    };

    Some(BatteryInfo { percentage, is_charging })
}

/// Volume information
#[derive(Debug, Clone)]
pub struct VolumeInfo {
    pub percentage: u8,
    pub muted: bool,
}

impl VolumeInfo {
    /// Get the appropriate icon for the volume level
    pub fn icon(&self) -> &'static str {
        if self.muted || self.percentage == 0 {
            return "󰖁";
        }
        match self.percentage {
            60..=100 => "󰕾",
            30..=59 => "󰖀",
            _ => "󰕿",
        }
    }
}

/// Get current volume information
pub fn get_volume() -> Option<VolumeInfo> {
    let output = Command::new("osascript")
        .args(["-e", "output volume of (get volume settings)"])
        .output()
        .ok()?;

    let volume_str = String::from_utf8_lossy(&output.stdout);
    let percentage = volume_str.trim().parse::<u8>().ok()?;

    // Check mute status
    let mute_output = Command::new("osascript")
        .args(["-e", "output muted of (get volume settings)"])
        .output()
        .ok()?;

    let muted = String::from_utf8_lossy(&mute_output.stdout)
        .trim()
        .eq_ignore_ascii_case("true");

    Some(VolumeInfo { percentage, muted })
}

/// Get current time formatted as DD/MM HH:MM
pub fn get_clock() -> String {
    let now = Local::now();
    now.format("%d/%m %H:%M").to_string()
}


/// Brew outdated information
#[derive(Debug, Clone, Default)]
pub struct BrewInfo {
    pub formulae: usize,
    pub casks: usize,
}

impl BrewInfo {
    /// Get the total count of outdated packages
    #[allow(dead_code)]
    pub fn total(&self) -> usize {
        self.formulae + self.casks
    }

    /// Get the appropriate icon
    pub fn icon(&self) -> &'static str {
        "\u{f487}"
    }
}

/// Get outdated brew formulae and casks count
pub fn get_brew_outdated() -> BrewInfo {
    let mut info = BrewInfo::default();

    // Get outdated formulae
    if let Ok(output) = Command::new("brew")
        .args(["outdated", "--formula", "-q"])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            info.formulae = stdout.lines().filter(|l| !l.is_empty()).count();
        }
    }

    // Get outdated casks
    if let Ok(output) = Command::new("brew")
        .args(["outdated", "--cask", "-q"])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            info.casks = stdout.lines().filter(|l| !l.is_empty()).count();
        }
    }

    info
}

/// CPU and RAM usage information
#[derive(Debug, Clone, Default)]
pub struct SystemInfo {
    pub cpu_percentage: u8,
    pub ram_percentage: u8,
    pub ram_used_gb: f32,
    pub ram_total_gb: f32,
}

impl SystemInfo {
    /// Get the CPU icon
    #[allow(dead_code)]
    pub fn cpu_icon(&self) -> &'static str {
        "\u{f0ee0}" // nf-md-cpu_64_bit
    }

    /// Get the RAM icon
    #[allow(dead_code)]
    pub fn ram_icon(&self) -> &'static str {
        "\u{f035b}" // nf-md-memory
    }
}

/// Get current CPU and RAM usage
pub fn get_system_info() -> SystemInfo {
    let mut info = SystemInfo::default();

    // Get CPU usage using top command
    if let Ok(output) = Command::new("top")
        .args(["-l", "1", "-n", "0"])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Parse CPU usage line: "CPU usage: 5.71% user, 3.57% sys, 90.71% idle"
            for line in stdout.lines() {
                if line.starts_with("CPU usage:") {
                    // Extract idle percentage and calculate usage
                    if let Some(idle_part) = line.split(',').nth(2) {
                        if let Some(idle_str) = idle_part.split('%').next() {
                            if let Ok(idle) = idle_str.trim().parse::<f32>() {
                                info.cpu_percentage = (100.0 - idle).round() as u8;
                            }
                        }
                    }
                    break;
                }
            }
        }
    }

    // Get RAM usage using sysinfo crate (much more efficient and accurate)
    let mut sys = System::new();
    sys.refresh_memory();
    
    let total_memory = sys.total_memory();
    let used_memory = sys.used_memory();
    
    if total_memory > 0 {
        info.ram_percentage = ((used_memory as f64 / total_memory as f64) * 100.0).round() as u8;
        info.ram_used_gb = (used_memory as f64 / 1_073_741_824.0) as f32;
        info.ram_total_gb = (total_memory as f64 / 1_073_741_824.0) as f32;
    }

    info
}

/// Microsoft Teams notification information
#[derive(Debug, Clone, Default)]
pub struct TeamsInfo {
    pub running: bool,
    pub notification_count: u32,
}

impl TeamsInfo {
    /// Get the appropriate icon (Microsoft Teams icon)
    pub fn icon(&self) -> &'static str {
        "󰊻" // nf-md-microsoft_teams
    }

    /// Get the icon color based on state
    pub fn icon_color(&self) -> &'static str {
        if !self.running {
            "0xff3c3836" // Same as active workspace bg when not running
        } else if self.notification_count > 0 {
            "0xfffabd2f" // Yellow/amber when notifications
        } else {
            "0xffffffff" // White (same as other icons)
        }
    }

    /// Get the border color based on state
    pub fn border_color(&self) -> &'static str {
        if self.notification_count > 0 {
            "0xfffabd2f" // Yellow/amber border for notifications
        } else {
            "0xff2a2c3a" // Default border
        }
    }
}

/// Get Microsoft Teams notification count
pub fn get_teams_notifications() -> TeamsInfo {
    let mut info = TeamsInfo::default();

    // Check if Teams is running (MSTeams is the new Teams app process name)
    let running = Command::new("pgrep")
        .args(["-x", "MSTeams"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    info.running = running;

    if !running {
        return info;
    }

    // Get notification count from Dock badge via AppleScript
    let script = r#"
tell application "System Events"
    tell UI element "Microsoft Teams" of list 1 of process "Dock"
        try
            set badgeValue to value of attribute "AXStatusLabel"
            if badgeValue is not missing value then
                return badgeValue
            end if
        end try
    end tell
end tell
return "0"
"#;

    if let Ok(output) = Command::new("osascript")
        .args(["-e", script])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Extract only digits from the result
            let count_str: String = stdout.trim().chars().filter(|c| c.is_ascii_digit()).collect();
            info.notification_count = count_str.parse().unwrap_or(0);
        }
    }

    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_battery_icons() {
        let high = BatteryInfo { percentage: 95, is_charging: false };
        assert_eq!(high.icon(), "󱊣");

        let is_charging = BatteryInfo { percentage: 50, is_charging: true };
        assert_eq!(is_charging.icon(), "\u{f0e7}"); // nf-fa-bolt

        let low = BatteryInfo { percentage: 5, is_charging: false };
        assert_eq!(low.icon(), "󰂎");
    }

    #[test]
    fn test_volume_icons() {
        let high = VolumeInfo { percentage: 80, muted: false };
        assert_eq!(high.icon(), "\u{f240}");

        let muted = VolumeInfo { percentage: 80, muted: true };
        assert_eq!(muted.icon(), "󰖁");

        let zero = VolumeInfo { percentage: 0, muted: false };
        assert_eq!(zero.icon(), "\u{f244}");
    }

    #[test]
    fn test_clock() {
        let clock = get_clock();
        assert!(clock.contains('/'));
        assert!(clock.contains(':'));
    }
}
