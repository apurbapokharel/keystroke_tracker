use evdev::KeyCode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

pub const CURRENT_VERSION: u8 = 1;

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Tracker {
    pub data: Mutex<TrackerState>,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct TrackerState {
    /// adding a version for backward compatibility and autoschema parrsing on the frontend
    pub version: u8,
    /// 0 is 12 am .... 24 is 11pm
    /// u16 is key_type.code(),
    /// u32 is times pressed
    pub count_freq: HashMap<u8, HashMap<u16, u32>>,
}

impl Tracker {
    pub fn new() -> Tracker {
        Tracker {
            data: Mutex::new(TrackerState::new()),
        }
    }
}

impl TrackerState {
    fn new() -> TrackerState {
        TrackerState {
            version: CURRENT_VERSION,
            count_freq: HashMap::new(),
        }
    }

    pub fn reset(&mut self) {
        self.count_freq = HashMap::new()
    }

    pub fn display(&self) {
        println!("Version {}", self.version);
        let mut total: u32 = 0;
        for (k, v) in &self.count_freq {
            println!("For hour {}", k);
            for (k2, v2) in v {
                println!("Key {:?}, Pressed {} times", KeyCode::new(*k2), v2);
                total += v2;
            }
        }
        println!("Total presses {}", total)
    }

    pub fn add_jsons(&mut self, current_state: &TrackerState) -> anyhow::Result<()> {
        for (hour, inner_map) in &current_state.count_freq {
            let entry = self.count_freq.entry(*hour).or_insert_with(HashMap::new);
            for (key, count) in inner_map {
                *entry.entry(*key).or_insert(0) += count;
            }
        }
        Ok(())
    }

    pub fn export_to_json(&self, path: &PathBuf, create_dir: bool) -> anyhow::Result<()> {
        if create_dir {
            // create the directories
            std::fs::create_dir_all(path).expect("failed to create directories");
        }
        // serizlize tracker_state to string
        let serialized = serde_json::to_string(self).expect("unable to serialize tracker_state");
        // save the serialized string to .json
        fs::write(path.join("keystrokes.json"), serialized)
            .expect("failed to write tracker_state into keystores.json");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_my_message_decoder() {
        let mut state = TrackerState::default();

        state.count_freq.insert(10, HashMap::new());
        state.count_freq.get_mut(&10).unwrap().insert(65, 3);

        for (k, v) in &state.count_freq {
            for (k2, v2) in v {
                println!("{}, {}, {}", k, k2, v2)
            }
        }
        let json = serde_json::to_string_pretty(&state.count_freq).unwrap();
        println!("json is {:?}", json);
        assert!(1 == 1, "assertion failed")
    }
}
