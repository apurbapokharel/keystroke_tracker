use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

pub const CURRENT_VERSION: u8 = 2;

pub const DATE_FMT: &str = "%Y-%m-%d";

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Tracker {
    // NOTE: i use a mutex here in the sturct rather than making the struct instanct a mutex.
    // I do this as per the recommendtaion in https://tokio.rs/tokio/tutorial/shared-state
    // TBH these tiny nuances are why I honestly feel you learn more by doing but doing is not
    // always easy, specially when it comes to rust.
    /// One `TrackerState` per calendar date, so a day that was never pushed
    /// keeps its own counters instead of folding into the next day's. One mutex
    /// guards the whole map: 
    pub data: Mutex<BTreeMap<String, TrackerState>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TrackerState {
    /// adding a version for backward compatibility and autoschema parrsing on the frontend
    #[serde(default)]
    pub version: u8,
    /// 0 is 12 am .... 24 is 11pm
    /// String is the evdev key name (e.g. "KEY_A", "KEY_SPACE"),
    /// u32 is times pressed.
    /// BTreeMap so hours (and keys) iterate in sorted order — deterministic
    /// `display()` output and stable on-disk JSON key ordering.
    #[serde(default)]
    pub keyboard_state: BTreeMap<u8, BTreeMap<String, u32>>,
    /// mouse_tracks
    #[serde(default)]
    pub mouse_state: MouseState,
    /// screen active session
    /// hours to active minute
    /// `#[serde(default)]` on every field: a JSON file written by an older
    /// schema (missing newer fields) deserializes with those fields defaulted
    /// instead of failing, so `push`/`add_jsons` can still merge it.
    #[serde(default)]
    pub display_state: BTreeMap<u8, u32>,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct MouseState {
    /// right_click count
    pub right_click: u32,
    /// left_click count
    pub left_click: u32,
    /// middle_click count
    pub middle_click: u32,
    /// AI: distance mouse pointer moved (physical desk travel, in inches).
    /// f64 rather than f32: this accumulates millions of tiny per-report
    /// increments over a day, and an f32 sum stops growing once it passes
    /// ~1e5 because the increments fall below its precision.
    pub mouse_inches: f64,
    /// number of mouse scrolls
    pub mouse_scrolls: u32,
}

impl Tracker {
    pub fn new() -> Tracker {
        Tracker {
            data: Mutex::new(BTreeMap::new()),
        }
    }

    /// Lock the shared state, recovering from a poisoned mutex.
    ///
    /// A poisoned lock only means some *other* thread panicked while holding the
    /// guard — the counters themselves are still coherent. For a long-running
    /// daemon it is better to keep tracking than to let one thread's panic
    /// cascade into every other thread via `.expect()`.
    pub fn state(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, TrackerState>> {
        self.data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// AI: Hand-written rather than derived so a bucket created by `or_default()`
/// carries `CURRENT_VERSION`.
impl Default for TrackerState {
    fn default() -> TrackerState {
        TrackerState {
            version: CURRENT_VERSION,
            keyboard_state: BTreeMap::new(),
            mouse_state: MouseState::default(),
            display_state: BTreeMap::new(),
        }
    }
}

impl TrackerState {
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

    /// Merge another state's counts into this one. Pure in-memory arithmetic —
    /// nothing here can fail, so it returns `()` rather than a `Result`.
    pub fn add_jsons(&mut self, current_state: &TrackerState) {
        // adding keyboard_state
        for (hour, inner_map) in &current_state.keyboard_state {
            let entry = self.keyboard_state.entry(*hour).or_default();
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
    }

    pub fn export_to_json(&self, path: &PathBuf, create_dir: bool) -> anyhow::Result<()> {
        if create_dir {
            // create the directories
            std::fs::create_dir_all(path)
                .with_context(|| format!("failed to create {}", path.display()))?;
        }
        // serizlize tracker_state to string
        let serialized = serde_json::to_string(self).context("unable to serialize tracker_state")?;
        // save the serialized string to .json
        let out = path.join("keystrokes.json");
        fs::write(&out, serialized)
            .with_context(|| format!("failed to write {}", out.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_my_message_decoder() {
        let mut state = TrackerState::default();

        state.keyboard_state.insert(10, BTreeMap::new());
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
    }
}
