use std::fs;
use std::path::PathBuf;

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

pub fn load_state() -> State {
    let path = session_path();
    if !path.exists() {
        return State::default();
    }
    match fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(state) => state,
            Err(_) => {
                eprintln!(
                    "warning: session file '{}' is corrupted; starting with empty state",
                    path.display()
                );
                State::default()
            }
        },
        Err(e) => {
            eprintln!("warning: could not read session file '{}': {}", path.display(), e);
            State::default()
        }
    }
}

pub fn save_state(state: &State) {
    let path = session_path();
    fs::create_dir_all(path.parent().unwrap()).expect("cannot create session directory");
    let content = serde_json::to_string_pretty(state).unwrap();
    fs::write(&path, content).expect("cannot write session file");
}

pub fn delete_session() {
    let path = session_path();
    if path.exists() {
        let _ = fs::remove_file(&path);
    }
}
