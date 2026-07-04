use super::*;

#[test]
#[cfg(any(unix, windows))]
fn ppid_is_nonzero() {
    assert!(get_ppid() > 0, "test runner must have a parent process");
}

fn temp_store() -> (FileSessionStore, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "reel_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("session.json");
    (FileSessionStore { path: path.clone() }, dir)
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
fn save_preset_stores_only_request_fields() {
    let dir = std::env::temp_dir().join(format!("reel_preset_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("preset.json");

    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());
    state.responses.push(crate::model::ResponseRecord {
        source: None,
        status: 200,
        headers: Default::default(),
        body: "body".to_string(),
    });

    save_preset(&state.request, &path).unwrap();
    let loaded = load_preset(&path).unwrap();
    assert!(loaded.responses.is_empty());
    assert_eq!(loaded.request.url, Some("https://example.com".to_string()));
    fs::remove_dir_all(dir).ok();
}
