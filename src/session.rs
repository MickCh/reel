use std::fs;
use std::path::{Path, PathBuf};

use crate::model::State;

// Uses the parent shell's PID as session key — each terminal window is isolated.
fn get_ppid() -> u32 {
    let ppid = fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("PPid:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|p| p.parse().ok())
        })
        .unwrap_or(0);
    if ppid == 0 {
        eprintln!("warning: could not determine parent PID; all processes will share session '0'");
    }
    ppid
}

pub fn session_path() -> PathBuf {
    dirs::home_dir()
        .expect("cannot determine home directory")
        .join(".reel")
        .join("sessions")
        .join(format!("{}.json", get_ppid()))
}

fn try_load(path: &std::path::Path) -> Result<State, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("could not read session file '{}': {}", path.display(), e))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("session file '{}' is corrupted ({}); starting with empty state", path.display(), e))
}

pub fn load_state() -> State {
    let path = session_path();
    if !path.exists() {
        return State::default();
    }
    match try_load(&path) {
        Ok(state) => state,
        Err(e) => {
            eprintln!("warning: {}", e);
            State::default()
        }
    }
}

pub fn save_preset(state: &State, path: &Path) -> Result<(), ()> {
    let mut to_save = state.clone();
    to_save.responses.clear();
    let content = serde_json::to_string_pretty(&to_save).unwrap();
    fs::write(path, content).map_err(|e| eprintln!("error writing '{}': {}", path.display(), e))
}

pub fn load_preset(path: &Path) -> Result<State, ()> {
    let content = fs::read_to_string(path)
        .map_err(|e| eprintln!("error reading '{}': {}", path.display(), e))?;
    serde_json::from_str(&content)
        .map_err(|e| eprintln!("error parsing '{}': {}", path.display(), e))
}

pub fn save_state(state: &State) {
    let path = session_path();
    fs::create_dir_all(path.parent().expect("session path has no parent"))
        .expect("cannot create session directory");
    let content = serde_json::to_string_pretty(state).unwrap();
    fs::write(&path, content).expect("cannot write session file");
}

pub fn delete_session() {
    let path = session_path();
    if path.exists() {
        let _ = fs::remove_file(&path);
    }
}
