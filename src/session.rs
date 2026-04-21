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

#[cfg(target_os = "linux")]
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

#[cfg(windows)]
fn get_ppid() -> u32 {
    use std::mem;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;

    *PPID.get_or_init(|| unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            eprintln!("warning: could not snapshot processes; all processes will share session '0'");
            return 0;
        }

        let current_pid = GetCurrentProcessId();
        let mut entry: PROCESSENTRY32W = mem::zeroed();
        entry.dwSize = mem::size_of::<PROCESSENTRY32W>() as u32;

        let mut ppid = 0u32;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                if entry.th32ProcessID == current_pid {
                    ppid = entry.th32ParentProcessID;
                    break;
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }

        CloseHandle(snapshot);

        if ppid == 0 {
            eprintln!("warning: could not determine parent PID; all processes will share session '0'");
        }
        ppid
    })
}

#[cfg(not(any(target_os = "linux", windows)))]
fn get_ppid() -> u32 {
    *PPID.get_or_init(|| {
        eprintln!("warning: parent PID lookup not supported on this platform; all processes will share session '0'");
        0
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

pub fn cleanup_old_sessions() {
    let dir = dirs::home_dir()
        .expect("cannot determine home directory")
        .join(".reel")
        .join("sessions");

    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(7 * 24 * 3600))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(metadata) = entry.metadata()
            && let Ok(modified) = metadata.modified()
            && modified < cutoff
        {
            let _ = fs::remove_file(&path);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(any(target_os = "linux", windows))]
    fn ppid_is_nonzero() {
        assert!(get_ppid() > 0, "test runner must have a parent process");
    }

    fn temp_store() -> (FileSessionStore, PathBuf) {
        let dir = std::env::temp_dir()
            .join(format!("reel_test_{}_{}", std::process::id(), std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.json");
        (FileSessionStore { path: path.clone() }, dir)
    }

    #[test]
    fn load_missing_returns_default() {
        let (store, dir) = temp_store();
        let state = store.load();
        assert!(state.method.is_none());
        assert!(state.url.is_none());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn save_and_load_round_trip() {
        let (store, dir) = temp_store();
        let mut state = State::default();
        state.method = Some("POST".to_string());
        state.url = Some("https://example.com".to_string());
        store.save(&state);

        let loaded = store.load();
        assert_eq!(loaded.method, Some("POST".to_string()));
        assert_eq!(loaded.url, Some("https://example.com".to_string()));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn delete_removes_file() {
        let (store, dir) = temp_store();
        store.save(&State::default());
        assert!(store.path.exists());
        store.delete();
        assert!(!store.path.exists());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn delete_is_noop_when_missing() {
        let (store, dir) = temp_store();
        store.delete(); // should not panic
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn corrupted_file_falls_back_to_default() {
        let (store, dir) = temp_store();
        fs::write(&store.path, "not valid json").unwrap();
        let state = store.load();
        assert!(state.method.is_none());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn save_preset_strips_responses() {
        let dir = std::env::temp_dir().join(format!("reel_preset_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("preset.json");

        let mut state = State::default();
        state.url = Some("https://example.com".to_string());
        state.responses.push(crate::model::ResponseRecord {
            source: None,
            status: 200,
            headers: Default::default(),
            body: "body".to_string(),
        });

        save_preset(&state, &path).unwrap();
        let loaded = load_preset(&path).unwrap();
        assert!(loaded.responses.is_empty());
        assert_eq!(loaded.url, Some("https://example.com".to_string()));
        fs::remove_dir_all(dir).ok();
    }
}
