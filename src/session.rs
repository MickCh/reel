use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::model::State;

pub trait SessionStore {
    fn load(&self) -> State;
    fn save(&self, state: &State);
    fn delete(&self);
    fn path(&self) -> &Path;
}

pub struct FileSessionStore {
    path: PathBuf,
}

impl FileSessionStore {
    pub fn new() -> Self {
        Self { path: make_session_path() }
    }
}

static PPID: OnceLock<u32> = OnceLock::new();

// Linux-specific: reads parent PID from /proc/self/status.
// Falls back to 0 (shared session) on other platforms or if the read fails.
fn get_ppid() -> u32 {
    *PPID.get_or_init(|| {
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
    })
}

fn make_session_path() -> PathBuf {
    dirs::home_dir()
        .expect("cannot determine home directory")
        .join(".reel")
        .join("sessions")
        .join(format!("{}.json", get_ppid()))
}

impl SessionStore for FileSessionStore {
    fn load(&self) -> State {
        if !self.path.exists() {
            return State::default();
        }
        match try_load(&self.path) {
            Ok(state) => state,
            Err(e) => {
                eprintln!("warning: {}", e);
                State::default()
            }
        }
    }

    fn save(&self, state: &State) {
        fs::create_dir_all(self.path.parent().expect("session path has no parent"))
            .expect("cannot create session directory");
        let content = serde_json::to_string_pretty(state).unwrap();
        fs::write(&self.path, content).expect("cannot write session file");
    }

    fn delete(&self) {
        if self.path.exists() {
            let _ = fs::remove_file(&self.path);
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

fn try_load(path: &Path) -> Result<State, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("could not read session file '{}': {}", path.display(), e))?;
    serde_json::from_str(&content).map_err(|e| {
        format!(
            "session file '{}' is corrupted ({}); starting with empty state",
            path.display(),
            e
        )
    })
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
