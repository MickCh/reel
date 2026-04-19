use std::fs;
use std::path::PathBuf;

use crate::model::State;

// Uses the parent shell's PID as session key — each terminal window is isolated.
fn get_ppid() -> u32 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("PPid:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|p| p.parse().ok())
        })
        .unwrap_or(0)
}

pub fn session_path() -> PathBuf {
    dirs::home_dir()
        .expect("cannot determine home directory")
        .join(".req")
        .join("sessions")
        .join(format!("{}.json", get_ppid()))
}

pub fn load_state() -> State {
    let path = session_path();
    if path.exists() {
        let content = fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        State::default()
    }
}

pub fn save_state(state: &State) {
    let path = session_path();
    fs::create_dir_all(path.parent().unwrap()).expect("cannot create session directory");
    let content = serde_json::to_string_pretty(state).unwrap();
    fs::write(&path, content).expect("cannot write session file");
}
