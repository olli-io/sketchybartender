use std::process::Command;
use std::thread;
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

    /// Get the icon color based on charging state, using colors from config
    pub fn icon_color<'a>(&self, config: &'a crate::config::Config) -> &'a str {
        if self.is_charging {
            &config.battery_charging_color // Charging
        } else if self.percentage <= 10 {
            &config.battery_low_color // Critically low
        } else {
            &config.battery_normal_color // Discharging normally
        }
    }

    pub fn label_color<'a>(&self, config: &'a crate::config::Config) -> &'a str {
        self.icon_color(config)
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
    // Run both brew commands in parallel for faster results
    let formulae_handle = thread::spawn(|| {
        Command::new("brew")
            .args(["outdated", "--formula", "-q"])
            .output()
            .ok()
            .and_then(|output| {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    Some(stdout.lines().filter(|l| !l.is_empty()).count())
                } else {
                    None
                }
            })
            .unwrap_or(0)
    });

    let casks_handle = thread::spawn(|| {
        Command::new("brew")
            .args(["outdated", "--cask", "-q"])
            .output()
            .ok()
            .and_then(|output| {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    Some(stdout.lines().filter(|l| !l.is_empty()).count())
                } else {
                    None
                }
            })
            .unwrap_or(0)
    });

    // Wait for both threads to complete
    let formulae = formulae_handle.join().unwrap_or(0);
    let casks = casks_handle.join().unwrap_or(0);

    BrewInfo {
        formulae,
        casks,
    }
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

/// A snapshot of cumulative CPU ticks since boot: (busy, total).
/// `busy` = user + system + nice; `total` = busy + idle.
pub type CpuTicks = (u64, u64);

/// Read the kernel's cumulative CPU-tick counters via `host_statistics`.
///
/// This is the same `HOST_CPU_LOAD_INFO` data Activity Monitor derives its CPU
/// figures from. A single call costs microseconds and — unlike `top` — spawns
/// no process and runs no sampling pass, so it doesn't pollute its own
/// measurement window (which is what made `top` over-report `sys`).
pub fn read_cpu_ticks() -> Option<CpuTicks> {
    use std::mem::MaybeUninit;

    let mut info = MaybeUninit::<libc::host_cpu_load_info>::uninit();
    let mut count = libc::HOST_CPU_LOAD_INFO_COUNT;
    let kr = unsafe {
        libc::host_statistics(
            mach2::mach_init::mach_host_self(),
            libc::HOST_CPU_LOAD_INFO,
            info.as_mut_ptr() as libc::host_info_t,
            &mut count,
        )
    };
    if kr != libc::KERN_SUCCESS {
        return None;
    }
    let info = unsafe { info.assume_init() };
    let ticks = &info.cpu_ticks;
    let user = ticks[libc::CPU_STATE_USER as usize] as u64;
    let sys = ticks[libc::CPU_STATE_SYSTEM as usize] as u64;
    let nice = ticks[libc::CPU_STATE_NICE as usize] as u64;
    let idle = ticks[libc::CPU_STATE_IDLE as usize] as u64;
    let busy = user + sys + nice;
    Some((busy, busy + idle))
}

/// Get current CPU and RAM usage.
///
/// CPU is measured as the delta between two `read_cpu_ticks()` snapshots. The
/// caller passes the previous snapshot (from the last refresh) and receives the
/// current one back, so the busy% is averaged over the whole refresh interval —
/// matching how Activity Monitor's CPU graph reads. The first call (no `prev`)
/// reports 0% and just returns the bootstrap snapshot.
pub fn get_system_info(prev_cpu: Option<CpuTicks>) -> (SystemInfo, Option<CpuTicks>) {
    let mut info = SystemInfo::default();

    // CPU usage from the tick-counter delta since the previous refresh.
    let cur_cpu = read_cpu_ticks();
    if let (Some((prev_busy, prev_total)), Some((cur_busy, cur_total))) = (prev_cpu, cur_cpu) {
        let busy = cur_busy.saturating_sub(prev_busy);
        let total = cur_total.saturating_sub(prev_total);
        if total > 0 {
            info.cpu_percentage = ((busy as f64 / total as f64) * 100.0).round() as u8;
        }
    }

    // Get RAM usage the way Activity Monitor reports it.
    //
    // macOS keeps almost nothing truly free — inactive, speculative and
    // purgeable pages are all reclaimable and are NOT counted as "used" by
    // Activity Monitor. We derive Activity Monitor's "Memory Used" =
    // App Memory + Wired + Compressed from the kernel's vm statistics:
    //   used = (active + wired + compressor - purgeable) * page_size
    let (used_bytes, total_bytes) = get_memory_used_total();
    if total_bytes > 0 {
        info.ram_percentage = ((used_bytes as f64 / total_bytes as f64) * 100.0).round() as u8;
        info.ram_used_gb = (used_bytes as f64 / 1_073_741_824.0) as f32;
        info.ram_total_gb = (total_bytes as f64 / 1_073_741_824.0) as f32;
    }

    (info, cur_cpu)
}

/// Total physical memory in bytes, from `sysctlbyname("hw.memsize")`.
/// This value is invariant for the life of the process.
fn total_memory_bytes() -> u64 {
    let mut value: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    let name = b"hw.memsize\0";
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr() as *const libc::c_char,
            &mut value as *mut u64 as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc == 0 {
        value
    } else {
        0
    }
}

/// Compute (used_bytes, total_bytes) matching Activity Monitor's "Memory Used",
/// reading the kernel's VM statistics directly via `host_statistics64` — no
/// `vm_stat`/`sysctl` subprocess.
fn get_memory_used_total() -> (u64, u64) {
    use std::mem::MaybeUninit;

    let total_bytes = total_memory_bytes();

    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    let page_size = if page_size > 0 { page_size as u64 } else { 4096 };

    let mut info = MaybeUninit::<libc::vm_statistics64>::uninit();
    let mut count = libc::HOST_VM_INFO64_COUNT;
    let kr = unsafe {
        libc::host_statistics64(
            mach2::mach_init::mach_host_self(),
            libc::HOST_VM_INFO64,
            info.as_mut_ptr() as libc::host_info64_t,
            &mut count,
        )
    };
    if kr != libc::KERN_SUCCESS {
        return (0, total_bytes);
    }
    let vm = unsafe { info.assume_init() };

    let used_pages = (vm.active_count as u64
        + vm.wire_count as u64
        + vm.compressor_page_count as u64)
        .saturating_sub(vm.purgeable_count as u64);

    (used_pages * page_size, total_bytes)
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
            "0xffF5EEE2" // White (same as other icons)
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
