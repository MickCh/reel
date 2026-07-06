use super::*;

#[test]
#[cfg(any(unix, windows))]
fn ppid_is_nonzero() {
    assert!(get_ppid() > 0, "test runner must have a parent process");
}

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "reel_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn temp_store() -> (FileSessionStore, PathBuf) {
    let dir = temp_dir();
    let path = dir.join("session.json");
    (
        FileSessionStore {
            path,
            owner_start_time: None,
        },
        dir,
    )
}

fn store_at(dir: &Path, owner_start_time: Option<u64>) -> FileSessionStore {
    FileSessionStore {
        path: dir.join("session.json"),
        owner_start_time,
    }
}

fn future_cutoff() -> SystemTime {
    SystemTime::now() + Duration::from_secs(3600)
}

#[test]
fn load_missing_returns_default() {
    let (store, dir) = temp_store();
    let state = store.load();
    assert!(state.request.method.is_none());
    assert!(state.request.url.is_none());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn save_and_load_round_trip() {
    let (store, dir) = temp_store();
    let mut state = State::default();
    state.request.method = Some("POST".to_string());
    state.request.url = Some("https://example.com".to_string());
    store.save(&state).unwrap();

    let loaded = store.load();
    assert_eq!(loaded.request.method, Some("POST".to_string()));
    assert_eq!(loaded.request.url, Some("https://example.com".to_string()));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn delete_removes_file() {
    let (store, dir) = temp_store();
    store.save(&State::default()).unwrap();
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
    assert!(state.request.method.is_none());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn save_leaves_no_temp_files() {
    let (store, dir) = temp_store();
    store.save(&State::default()).unwrap();
    let leftovers: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("tmp"))
        .collect();
    assert!(leftovers.is_empty());
    fs::remove_dir_all(dir).ok();
}

#[test]
#[cfg(unix)]
fn session_file_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let (store, dir) = temp_store();
    store.save(&State::default()).unwrap();
    let mode = fs::metadata(&store.path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn stale_owner_stamp_starts_fresh() {
    let dir = temp_dir();
    let old = store_at(&dir, Some(1));
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());
    old.save(&state).unwrap();

    // Same path, different owner start time — simulates a recycled PID.
    let new = store_at(&dir, Some(2));
    let loaded = new.load();
    assert!(loaded.request.url.is_none());
    assert!(!new.path.exists(), "stale session file should be removed");
    fs::remove_dir_all(dir).ok();
}

#[test]
fn matching_owner_stamp_keeps_state() {
    let dir = temp_dir();
    let store = store_at(&dir, Some(42));
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());
    store.save(&state).unwrap();

    let again = store_at(&dir, Some(42));
    let loaded = again.load();
    assert_eq!(loaded.request.url, Some("https://example.com".to_string()));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn unstamped_file_is_accepted() {
    // Session file written by an older reel version (no owner_start_time).
    let dir = temp_dir();
    let unstamped = store_at(&dir, None);
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());
    unstamped.save(&state).unwrap();

    let stamped = store_at(&dir, Some(7));
    let loaded = stamped.load();
    assert_eq!(loaded.request.url, Some("https://example.com".to_string()));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn stamp_ignored_when_current_owner_unknown() {
    let dir = temp_dir();
    let stamped = store_at(&dir, Some(7));
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());
    stamped.save(&state).unwrap();

    let unknown = store_at(&dir, None);
    let loaded = unknown.load();
    assert_eq!(loaded.request.url, Some("https://example.com".to_string()));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn session_name_validation() {
    assert!(is_valid_session_name("work"));
    assert!(is_valid_session_name("ci-run_42.x"));
    assert!(!is_valid_session_name(""));
    assert!(!is_valid_session_name("has space"));
    assert!(!is_valid_session_name("../escape"));
    assert!(!is_valid_session_name(&"x".repeat(65)));
}

#[test]
#[cfg(target_os = "linux")]
fn start_time_of_current_process_is_known() {
    assert!(process_start_time(std::process::id()).is_some());
}

#[test]
#[cfg(any(unix, windows))]
fn process_liveness() {
    assert_eq!(process_alive(std::process::id()), Some(true));
    // Far beyond any real PID range on Linux/macOS/Windows.
    assert_eq!(process_alive(999_999_999), Some(false));
}

#[test]
#[cfg(any(unix, windows))]
fn cleanup_removes_dead_pid_sessions() {
    let dir = temp_dir();
    let dead = dir.join("999999999.json");
    fs::write(&dead, "{}").unwrap();
    // Cutoff in the distant past: age alone would keep the file.
    cleanup_dir(&dir, SystemTime::UNIX_EPOCH);
    assert!(!dead.exists());
    fs::remove_dir_all(dir).ok();
}

#[test]
#[cfg(any(unix, windows))]
fn cleanup_keeps_live_pid_sessions_regardless_of_age() {
    let dir = temp_dir();
    let alive = dir.join(format!("{}.json", std::process::id()));
    fs::write(&alive, "{}").unwrap();
    // Cutoff in the future: age alone would delete the file.
    cleanup_dir(&dir, future_cutoff());
    assert!(alive.exists());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn cleanup_applies_age_rule_to_unverifiable_files() {
    let dir = temp_dir();
    let named = dir.join("named-work.json");
    let zero = dir.join("0.json");
    let tmp = dir.join("session.123.tmp");
    let other = dir.join("notes.txt");
    for path in [&named, &zero, &tmp, &other] {
        fs::write(path, "{}").unwrap();
    }

    cleanup_dir(&dir, SystemTime::UNIX_EPOCH);
    assert!(named.exists() && zero.exists() && tmp.exists() && other.exists());

    cleanup_dir(&dir, future_cutoff());
    assert!(!named.exists() && !zero.exists() && !tmp.exists());
    assert!(other.exists(), "non-session files are never touched");
    fs::remove_dir_all(dir).ok();
}

#[test]
fn save_preset_stores_only_request_fields() {
    let dir = std::env::temp_dir().join(format!("reel_preset_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("preset.json");

    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());
    state.responses.push(crate::model::ResponseRecord {
        status: 200,
        body: "body".to_string(),
        ..Default::default()
    });

    let presets = FilePresetStore;
    presets.save(&state.request, &path).unwrap();
    let loaded = presets.load(&path).unwrap();
    assert!(loaded.responses.is_empty());
    assert_eq!(loaded.request.url, Some("https://example.com".to_string()));
    fs::remove_dir_all(dir).ok();
}
