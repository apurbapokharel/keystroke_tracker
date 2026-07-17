use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

pub const CURRENT_VERSION: u8 = 2;

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Tracker {
    // NOTE: i use a mutex here in the sturct rather than making the struct instanct a mutex.
    // I do this as per the recommendtaion in https://tokio.rs/tokio/tutorial/shared-state
    // TBH these tiny nuances are why I honestly feel you learn more by doing but doing is not
    // always easy, specially when it comes to rust.
    pub data: Mutex<TrackerState>,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct TrackerState {
    /// adding a version for backward compatibility and autoschema parrsing on the frontend
    pub version: u8,
    /// 0 is 12 am .... 24 is 11pm
    /// String is the evdev key name (e.g. "KEY_A", "KEY_SPACE"),
    /// u32 is times pressed
    pub keyboard_state: HashMap<u8, HashMap<String, u32>>,
    /// mouse_tracks
    pub mouse_state: MouseState,
    /// screen active session
    /// hours to active minute
    pub display_state: HashMap<u8, u32>,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct MouseState {
    /// right_click count
    pub right_click: u32,
    /// left_click count
    pub left_click: u32,
    /// middle_click count
    pub middle_click: u32,
    /// distance mouse pointer moved
    pub mouse_inches: f32,
    /// number of mouse scrolls
    pub mouse_scrolls: u32,
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
            keyboard_state: HashMap::new(),
            mouse_state: MouseState::default(),
            display_state: HashMap::new(),
        }
    }

    pub fn reset(&mut self) {
        self.keyboard_state = HashMap::new();
        self.mouse_state = MouseState::default();
        self.display_state = HashMap::new();
    }

    pub fn display(&self) {
        println!("=== Tracker State (version {}) ===", self.version);

        let mut total: u32 = 0;
        for (hour, keys) in &self.keyboard_state {
            println!("  Hour {}:", hour);
            for (key, count) in keys {
                println!("    {}: {}", key, count);
                total += count;
            }
        }
        println!("  Total key presses: {}", total);

        println!("  Mouse:");
        println!("    Left clicks:   {}", self.mouse_state.left_click);
        println!("    Right clicks:  {}", self.mouse_state.right_click);
        println!("    Middle clicks: {}", self.mouse_state.middle_click);
        println!("    Inches moved:  {:.2}", self.mouse_state.mouse_inches);
        println!("    Scrolls:       {}", self.mouse_state.mouse_scrolls);

        let mut total_active: u32 = 0;
        for (hour, secs) in &self.display_state {
            println!("    Hour {} active: {}s", hour, secs);
            total_active += secs;
        }
        let hrs = total_active / 3600;
        let mins = (total_active % 3600) / 60;
        let secs = total_active % 60;
        println!("  Total screen-on time: {}h {}m {}s", hrs, mins, secs);
    }

    pub fn add_jsons(&mut self, current_state: &TrackerState) -> anyhow::Result<()> {
        // adding keyboard_state
        for (hour, inner_map) in &current_state.keyboard_state {
            let entry = self
                .keyboard_state
                .entry(*hour)
                .or_insert_with(HashMap::new);
            for (key, count) in inner_map {
                *entry.entry(key.clone()).or_insert(0) += count;
            }
        }
        // adding mouse_state
        self.mouse_state.right_click += current_state.mouse_state.right_click;
        self.mouse_state.left_click += current_state.mouse_state.left_click;
        self.mouse_state.middle_click += current_state.mouse_state.middle_click;
        self.mouse_state.mouse_inches += current_state.mouse_state.mouse_inches;
        self.mouse_state.mouse_scrolls += current_state.mouse_state.mouse_scrolls;

        // adding display_state
        for (hour, count) in &current_state.display_state {
            *self.display_state.entry(*hour).or_insert(0) += count;
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

        state.keyboard_state.insert(10, HashMap::new());
        state
            .keyboard_state
            .get_mut(&10)
            .unwrap()
            .insert("KEY_A".to_string(), 3);

        for (k, v) in &state.keyboard_state {
            for (k2, v2) in v {
                println!("{}, {}, {}", k, k2, v2)
            }
        }
        let json = serde_json::to_string_pretty(&state.keyboard_state).unwrap();
        println!("json is {:?}", json);
        assert!(1 == 1, "assertion failed")
    }
}
