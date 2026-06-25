use evdev::KeyCode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

    pub fn display(&self) {
        println!("Version {}", self.version);
        for (k, v) in &self.count_freq {
            println!("For hour {}", k);
            for (k2, v2) in v {
                println!("Key {:?}, Pressed {} times", KeyCode::new(*k2), v2)
            }
        }
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
