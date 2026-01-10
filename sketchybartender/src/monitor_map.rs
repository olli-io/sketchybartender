use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Cache entry for display mappings only (workspaces are queried fresh each time)
#[derive(Debug, Clone)]
struct CacheEntry {
    /// Map of Sketchybar display ID -> Aerospace monitor ID
    mappings: HashMap<u32, u32>,
    /// When this cache was created
    created_at: Instant,
}

impl CacheEntry {
    fn is_expired(&self, ttl: Duration) -> bool {
        self.created_at.elapsed() > ttl
    }
}

/// Manages the mapping between Sketchybar displays and Aerospace monitors
#[derive(Debug)]
pub struct MonitorMapper {
    cache: Arc<Mutex<Option<CacheEntry>>>,
    cache_ttl: Duration,
}

impl MonitorMapper {
    /// Create a new monitor mapper with a cache TTL (default: 5 minutes)
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(None)),
            cache_ttl: Duration::from_secs(300), // 5 minutes
        }
    }

    /// Get the mapping of Sketchybar display ID -> Aerospace monitor ID
    pub fn get_mappings(&self) -> HashMap<u32, u32> {
        // Check cache first
        if let Ok(cache) = self.cache.lock() {
            if let Some(entry) = cache.as_ref() {
                if !entry.is_expired(self.cache_ttl) {
                    return entry.mappings.clone();
                }
            }
        }

        // Cache miss or expired - rebuild mappings
        let mappings = self.build_mappings();

        // Update cache
        if let Ok(mut cache) = self.cache.lock() {
            *cache = Some(CacheEntry {
                mappings: mappings.clone(),
                created_at: Instant::now(),
            });
        }

        mappings
    }


    /// Build the monitor mappings from scratch (display mappings only)
    fn build_mappings(&self) -> HashMap<u32, u32> {
        let mut mappings = HashMap::new();

        // Get NSScreen data (NSScreen ID -> NSScreen Name)
        let nsscreen_map = self.get_nsscreen_map();

        // Get Sketchybar data (Sketchybar ID -> NSScreen ID)
        let sketchybar_map = self.get_sketchybar_map();

        // Get Aerospace data (Aerospace ID -> NSScreen Name)
        let aerospace_map = self.get_aerospace_map();

        // Build the mapping: Sketchybar ID -> Aerospace ID
        for (sb_id, nsscreen_id) in sketchybar_map {
            // Find NSScreen name for this NSScreen ID
            if let Some(nsscreen_name) = nsscreen_map.get(&nsscreen_id) {
                // Find Aerospace ID for this NSScreen name
                for (aero_id, aero_name) in &aerospace_map {
                    if aero_name == nsscreen_name {
                        mappings.insert(sb_id, *aero_id);
                        break;
                    }
                }
            }
        }

        mappings
    }

    /// Get NSScreen ID -> NSScreen Name mapping
    fn get_nsscreen_map(&self) -> HashMap<u32, String> {
        let mut map = HashMap::new();

        let swift_code = r#"import AppKit; for screen in NSScreen.screens { if let screenID = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber { print("\(screenID.intValue)|\(screen.localizedName)") } }"#;

        if let Ok(output) = Command::new("swift")
            .arg("-e")
            .arg(swift_code)
            .output()
        {
            if output.status.success() {
                for line in String::from_utf8_lossy(&output.stdout).lines() {
                    let parts: Vec<&str> = line.splitn(2, '|').collect();
                    if parts.len() == 2 {
                        if let Ok(id) = parts[0].parse::<u32>() {
                            map.insert(id, parts[1].to_string());
                        }
                    }
                }
            }
        }

        map
    }

    /// Get Sketchybar ID -> NSScreen ID mapping
    fn get_sketchybar_map(&self) -> HashMap<u32, u32> {
        let mut map = HashMap::new();

        if let Ok(output) = Command::new("sketchybar")
            .args(["--query", "displays"])
            .output()
        {
            if output.status.success() {
                // Parse JSON output
                if let Ok(json) = String::from_utf8(output.stdout) {
                    // Parse manually to avoid adding serde dependency
                    // Expected format: [{"arrangement-id":1,"DirectDisplayID":3},...]
                    // The JSON spans multiple lines, so we need to work with the whole string

                    // Find all arrangement-id and DirectDisplayID pairs
                    let mut arr_id: Option<u32> = None;
                    let mut disp_id: Option<u32> = None;

                    for line in json.lines() {
                        let trimmed = line.trim();

                        // Reset when we see a new object start
                        if trimmed.starts_with('{') {
                            arr_id = None;
                            disp_id = None;
                        }

                        if let Some(id) = self.extract_json_number(&line, "arrangement-id") {
                            arr_id = Some(id);
                        }
                        if let Some(id) = self.extract_json_number(&line, "DirectDisplayID") {
                            disp_id = Some(id);
                        }

                        // When we see object end and have both values, add to map
                        if trimmed.ends_with("},") || trimmed.ends_with('}') {
                            if let (Some(a), Some(d)) = (arr_id, disp_id) {
                                map.insert(a, d);
                            }
                        }
                    }
                }
            }
        }

        map
    }

    /// Get Aerospace monitor ID -> NSScreen Name mapping
    fn get_aerospace_map(&self) -> HashMap<u32, String> {
        let mut map = HashMap::new();

        if let Ok(output) = Command::new("aerospace")
            .args(["list-monitors", "--format", "%{monitor-id}|%{monitor-name}"])
            .output()
        {
            if output.status.success() {
                for line in String::from_utf8_lossy(&output.stdout).lines() {
                    let parts: Vec<&str> = line.splitn(2, '|').collect();
                    if parts.len() == 2 {
                        if let Ok(id) = parts[0].parse::<u32>() {
                            map.insert(id, parts[1].to_string());
                        }
                    }
                }
            }
        }

        map
    }

    /// Extract a number value from a JSON string (simple parser, no dependencies)
    fn extract_json_number(&self, json: &str, key: &str) -> Option<u32> {
        let search = format!("\"{}\":", key);
        if let Some(pos) = json.find(&search) {
            let after = &json[pos + search.len()..];
            let num_str: String = after
                .chars()
                .skip_while(|c| c.is_whitespace())
                .take_while(|c| c.is_numeric())
                .collect();
            num_str.parse().ok()
        } else {
            None
        }
    }

    /// Invalidate the cache (useful when monitors are added/removed)
    #[allow(dead_code)]
    pub fn invalidate_cache(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            *cache = None;
        }
    }
}

impl Default for MonitorMapper {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_expiration() {
        let entry = CacheEntry {
            mappings: HashMap::new(),
            created_at: Instant::now() - Duration::from_secs(400),
        };
        assert!(entry.is_expired(Duration::from_secs(300)));
    }

    #[test]
    fn test_cache_not_expired() {
        let entry = CacheEntry {
            mappings: HashMap::new(),
            created_at: Instant::now(),
        };
        assert!(!entry.is_expired(Duration::from_secs(300)));
    }

    #[test]
    fn test_json_number_extraction() {
        let mapper = MonitorMapper::new();
        let json = r#"{"arrangement-id": 1, "DirectDisplayID": 3}"#;
        assert_eq!(mapper.extract_json_number(json, "arrangement-id"), Some(1));
        assert_eq!(mapper.extract_json_number(json, "DirectDisplayID"), Some(3));
    }

    #[test]
    fn test_sketchybar_map_parsing() {
        let mapper = MonitorMapper::new();
        // Simulate actual sketchybar JSON output format (multi-line)
        let json = r#"[
	{
		"arrangement-id":1,
		"DirectDisplayID":3,
		"UUID":"test1"
	},
	{
		"arrangement-id":2,
		"DirectDisplayID":2,
		"UUID":"test2"
	}
]"#;
        // Test the parsing logic manually
        let mut map = HashMap::new();
        let mut arr_id: Option<u32> = None;
        let mut disp_id: Option<u32> = None;

        for line in json.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('{') {
                arr_id = None;
                disp_id = None;
            }
            if let Some(id) = mapper.extract_json_number(&line, "arrangement-id") {
                arr_id = Some(id);
            }
            if let Some(id) = mapper.extract_json_number(&line, "DirectDisplayID") {
                disp_id = Some(id);
            }
            if trimmed.ends_with("},") || trimmed.ends_with('}') {
                if let (Some(a), Some(d)) = (arr_id, disp_id) {
                    map.insert(a, d);
                }
            }
        }

        assert_eq!(map.get(&1), Some(&3));
        assert_eq!(map.get(&2), Some(&2));
        assert_eq!(map.len(), 2);
    }
}
