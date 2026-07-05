use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::model::{Request, State};

pub trait SessionStore {
    fn load(&self) -> State;
    fn save(&self, state: &State) -> Result<()>;
    fn delete(&self);
    fn path(&self) -> &Path;
}

// Preset files: user-addressed request snapshots loaded by `load`/`then` and
// written by `save`. Separate from SessionStore — presets live at caller-given
// paths, sessions at keyed paths the store owns.
pub trait PresetStore {
    fn load(&self, path: &Path) -> Result<State>;
    fn save(&self, request: &Request, path: &Path) -> Result<()>;
}

pub struct FileSessionStore {
    path: PathBuf,
    // Start time of the owning parent process. Some only for PPID-keyed
    // sessions on platforms where it can be read; used to detect PID reuse.
    owner_start_time: Option<u64>,
}

impl FileSessionStore {
    pub fn new() -> Result<Self> {
        let dir = sessions_dir()?;
        if let Some(name) = named_session() {
            return Ok(Self {
                path: dir.join(format!("named-{}.json", name)),
                owner_start_time: None,
            });
        }
        let ppid = get_ppid();
        Ok(Self {
            path: dir.join(format!("{}.json", ppid)),
            owner_start_time: process_start_time(ppid),
        })
    }
}

fn sessions_dir() -> Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("error: cannot determine home directory"))?;
    Ok(home.join(".reel").join("sessions"))
}

// REEL_SESSION overrides the PPID-based session key, letting scripts and
// wrappers share a session with (or isolate one from) the invoking shell.
// Named session files are prefixed so they can never collide with PID files.
fn named_session() -> Option<String> {
    let name = std::env::var("REEL_SESSION").ok()?;
    if is_valid_session_name(&name) {
        Some(name)
    } else {
        eprintln!(
            "warning: ignoring invalid REEL_SESSION '{}' (1-64 chars from A-Za-z0-9._-); using parent PID session",
            name
        );
        None
    }
}

fn is_valid_session_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
}

static PPID: OnceLock<u32> = OnceLock::new();

#[cfg(unix)]
fn get_ppid() -> u32 {
    *PPID.get_or_init(|| unsafe { libc::getppid() as u32 })
}

#[cfg(windows)]
fn get_ppid() -> u32 {
    use std::mem;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;

    *PPID.get_or_init(|| unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            eprintln!(
                "warning: could not snapshot processes; all processes will share session '0'"
            );
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
            eprintln!(
                "warning: could not determine parent PID; all processes will share session '0'"
            );
        }
        ppid
    })
}

#[cfg(not(any(unix, windows)))]
fn get_ppid() -> u32 {
    *PPID.get_or_init(|| {
        eprintln!("warning: parent PID lookup not supported on this platform; all processes will share session '0'");
        0
    })
}

// Start time of the given process, in platform-specific units (clock ticks on
// Linux, FILETIME on Windows). Only equality is ever checked, so the unit
// does not matter. None when the platform offers no way to read it.
#[cfg(target_os = "linux")]
fn process_start_time(pid: u32) -> Option<u64> {
    // /proc/<pid>/stat field 22 (starttime). The comm field (2) may contain
    // spaces and parentheses, so split after the *last* ')': the remaining
    // whitespace-separated tokens start at field 3.
    let stat = fs::read_to_string(format!("/proc/{}/stat", pid)).ok()?;
    let after_comm = stat.rsplit_once(')')?.1;
    after_comm.split_whitespace().nth(19)?.parse().ok()
}

#[cfg(windows)]
fn process_start_time(pid: u32) -> Option<u64> {
    use std::mem;
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut creation: FILETIME = mem::zeroed();
        let mut exit: FILETIME = mem::zeroed();
        let mut kernel: FILETIME = mem::zeroed();
        let mut user: FILETIME = mem::zeroed();
        let ok = GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user);
        CloseHandle(handle);
        if ok == 0 {
            return None;
        }
        Some(((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64)
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
fn process_start_time(_pid: u32) -> Option<u64> {
    None
}

// Whether a process with the given PID currently exists. None when it cannot
// be determined (the caller falls back to an age-based heuristic).
#[cfg(unix)]
fn process_alive(pid: u32) -> Option<bool> {
    if pid == 0 || pid > i32::MAX as u32 {
        return None;
    }
    if unsafe { libc::kill(pid as i32, 0) } == 0 {
        return Some(true);
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::EPERM) => Some(true), // exists, owned by someone else
        Some(libc::ESRCH) => Some(false),
        _ => None,
    }
}

#[cfg(windows)]
fn process_alive(pid: u32) -> Option<bool> {
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, GetLastError, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if !handle.is_null() {
            // The handle may refer to a terminated process that is still
            // held open elsewhere — check it is actually running.
            let mut code = 0u32;
            let alive = GetExitCodeProcess(handle, &mut code) != 0 && code == STILL_ACTIVE as u32;
            CloseHandle(handle);
            return Some(alive);
        }
        match GetLastError() {
            ERROR_ACCESS_DENIED => Some(true), // exists, no access rights
            ERROR_INVALID_PARAMETER => Some(false),
            _ => None,
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn process_alive(_pid: u32) -> Option<bool> {
    None
}

// On-disk session file: the State plus an ownership stamp. The stamp records
// the start time of the parent shell the session belongs to, so a recycled
// PID does not inherit a previous shell's session. Named (REEL_SESSION)
// sessions carry no stamp. Unknown fields are ignored during State
// deserialization, so old session files and preset files are unaffected.
#[derive(Deserialize)]
struct SessionFile {
    #[serde(default)]
    owner_start_time: Option<u64>,
    #[serde(flatten)]
    state: State,
}

#[derive(Serialize)]
struct SessionFileRef<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    owner_start_time: Option<u64>,
    #[serde(flatten)]
    state: &'a State,
}

impl SessionStore for FileSessionStore {
    fn load(&self) -> State {
        if !self.path.exists() {
            return State::default();
        }
        match try_load(&self.path) {
            Ok(file) => {
                // A stamp mismatch means the file was left behind by an
                // earlier process that happened to have the same PID: this
                // is a brand-new session, so start clean.
                if let (Some(current), Some(recorded)) =
                    (self.owner_start_time, file.owner_start_time)
                    && current != recorded
                {
                    let _ = fs::remove_file(&self.path);
                    return State::default();
                }
                file.state
            }
            Err(e) => {
                eprintln!("warning: {}", e);
                State::default()
            }
        }
    }

    fn save(&self, state: &State) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            create_session_dir(dir).map_err(|e| {
                anyhow::anyhow!(
                    "error: cannot create session directory '{}': {}",
                    dir.display(),
                    e
                )
            })?;
        }
        let content = serde_json::to_string_pretty(&SessionFileRef {
            owner_start_time: self.owner_start_time,
            state,
        })?;
        // Write to a temp file and rename over the target — an interrupted
        // save can never leave a corrupted session file. The PID in the temp
        // name keeps concurrent invocations from clobbering each other's
        // half-written data.
        let tmp = self
            .path
            .with_extension(format!("{}.tmp", std::process::id()));
        write_private(&tmp, &content).map_err(|e| {
            anyhow::anyhow!(
                "error: cannot write session file '{}': {}",
                tmp.display(),
                e
            )
        })?;
        fs::rename(&tmp, &self.path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            anyhow::anyhow!(
                "error: cannot write session file '{}': {}",
                self.path.display(),
                e
            )
        })
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

// Session files can hold credentials (Authorization headers, token bodies) —
// keep the directory and files private on Unix.
#[cfg(unix)]
fn create_session_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_session_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)
}

#[cfg(unix)]
fn write_private(path: &Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(content.as_bytes())
}

#[cfg(not(unix))]
fn write_private(path: &Path, content: &str) -> std::io::Result<()> {
    fs::write(path, content)
}

fn try_load(path: &Path) -> Result<SessionFile, String> {
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

// Age threshold for session files whose owning process cannot be verified
// (named sessions, the PID-0 fallback, platforms without a liveness check)
// and for leftover .tmp files from interrupted saves.
const MAX_UNVERIFIED_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

pub fn cleanup_old_sessions() {
    let Ok(dir) = sessions_dir() else { return };
    let cutoff = SystemTime::now()
        .checked_sub(MAX_UNVERIFIED_AGE)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    cleanup_dir(&dir, cutoff);
}

// PID-keyed session files are removed as soon as their process is gone —
// a session should not outlive its shell, and a live shell's session is
// never removed no matter how old (think week-long tmux sessions). Files
// whose owner cannot be verified fall back to the age rule.
fn cleanup_dir(dir: &Path, cutoff: SystemTime) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let stale = match path.extension().and_then(|e| e.to_str()) {
            Some("json") => session_file_stale(&path, &entry, cutoff),
            Some("tmp") => older_than(&entry, cutoff),
            _ => false,
        };
        if stale {
            let _ = fs::remove_file(&path);
        }
    }
}

fn session_file_stale(path: &Path, entry: &fs::DirEntry, cutoff: SystemTime) -> bool {
    let pid = path
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.parse::<u32>().ok());
    match pid {
        Some(pid) if pid != 0 => match process_alive(pid) {
            Some(alive) => !alive,
            None => older_than(entry, cutoff),
        },
        _ => older_than(entry, cutoff),
    }
}

fn older_than(entry: &fs::DirEntry, cutoff: SystemTime) -> bool {
    matches!(
        entry.metadata().and_then(|m| m.modified()),
        Ok(modified) if modified < cutoff
    )
}

pub struct FilePresetStore;

impl PresetStore for FilePresetStore {
    fn load(&self, path: &Path) -> Result<State> {
        let content = fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("error reading '{}': {}", path.display(), e))?;
        serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("error parsing '{}': {}", path.display(), e))
    }

    // Presets store only the request fields — never the request/response history.
    fn save(&self, request: &Request, path: &Path) -> Result<()> {
        let content = serde_json::to_string_pretty(request)?;
        fs::write(path, content)
            .map_err(|e| anyhow::anyhow!("error writing '{}': {}", path.display(), e))
    }
}

#[cfg(test)]
mod tests;
