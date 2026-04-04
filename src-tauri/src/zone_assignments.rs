use crate::config::PaveConfig;
use crate::zone_layout::ZoneLeafId;
use std::collections::HashMap;
use std::fs;

/// Persistent mapping of window class → zone leaf ID.
/// Tracks where the user last tiled each app so new windows
/// can be auto-placed into the same zone.
pub struct ZoneAssignments {
    map: HashMap<String, String>,
}

impl ZoneAssignments {
    /// Load assignments from disk, returning empty map on failure.
    pub fn load() -> Self {
        let path = PaveConfig::config_dir().join("zone_assignments.json");
        let map = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { map }
    }

    /// Save assignments to disk.
    fn save(&self) {
        let path = PaveConfig::config_dir().join("zone_assignments.json");
        if let Ok(json) = serde_json::to_string_pretty(&self.map) {
            if let Err(e) = fs::write(&path, json) {
                log::error!("Failed to save zone assignments: {e}");
            }
        }
    }

    /// Record that a window class was tiled into a zone.
    pub fn set(&mut self, resource_class: &str, leaf_id: &ZoneLeafId) {
        let key = resource_class.to_lowercase();
        if key.is_empty() {
            return;
        }
        self.map.insert(key, leaf_id.0.clone());
        self.save();
    }

    /// Look up the remembered zone for a window class.
    pub fn get(&self, resource_class: &str) -> Option<ZoneLeafId> {
        let key = resource_class.to_lowercase();
        self.map.get(&key).map(|s| ZoneLeafId(s.clone()))
    }
}
