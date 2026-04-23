# reel — developer notes for Claude

## Project overview

`reel` is a stateful CLI HTTP client written in Rust. It behaves like `curl` but persists request state (method, URL, headers, body) across invocations within the same terminal session.

## Architecture

Modules under `src/`:

| File | Responsibility |
|---|---|
| `main.rs` | Entry point — wires modules together |
| `model/mod.rs` | `State`, `ResponseRecord` structs (pure data, no I/O) |
| `session/mod.rs` | `SessionStore` trait + `FileSessionStore` impl; `load_preset`/`save_preset`/`cleanup_old_sessions` free functions |
| `template/mod.rs` | Template interpolation engine (`${{ expr }}`) |
| `http.rs` | `HttpClient` trait + `ReqwestClient` impl |
| `cli/mod.rs` | Re-exports public API of the `cli` submodules |
| `cli/commands.rs` | `Command` enum, `ResponseTarget`, `ResponseView`, `GlobalFlags`, `ParseResult` types |
| `cli/parser.rs` | `parse_args` and response target/view parsing helpers |
| `cli/display.rs` | All display functions: `show_state`, `show_response`, `print_usage`, `format_status`, etc. |
| `cli/runner.rs` | `run_commands` — command execution loop |

Each module with tests has a companion `tests.rs` file (e.g. `cli/tests.rs`, `model/tests.rs`) declared via `#[cfg(test)] mod tests;` — tests are compiled as part of the module, retaining access to private items.

Dependency direction: `cli` → `http`/`template`/`session` → `model`. No module depends on a layer above it.

### Key traits

**`http::HttpClient`** — abstracts HTTP execution. `run_commands` receives `&dyn HttpClient`, making it testable without real network calls.

```rust
pub trait HttpClient {
    fn execute(&self, state: &State, source: Option<&str>) -> Result<ResponseRecord>;
}
```

**`session::SessionStore`** — abstracts session persistence. `run_commands` receives `&dyn SessionStore`.

```rust
pub trait SessionStore {
    fn load(&self) -> State;
    fn save(&self, state: &State);
    fn delete(&self);
    fn path(&self) -> &Path;
}
```

`main.rs` constructs the concrete implementations (`ReqwestClient`, `FileSessionStore`) and passes them in.

### Session identification

Uses the parent shell PID as the session key. State is stored in `~/.reel/sessions/<ppid>.json`. The PPID is read once via `OnceLock<u32>` and cached for the process lifetime.

Platform-specific PPID lookup is gated with `#[cfg(...)]` inside `get_ppid()` in `session/mod.rs`:

| Platform | Mechanism |
|---|---|
| Unix (Linux, macOS, BSD, …) | `libc::getppid()` |
| Windows | `CreateToolhelp32Snapshot` + `Process32FirstW` from `windows-sys` |
| other | Falls back to `0` with a warning (all invocations share one session) |

The `SessionStore` trait itself is platform-neutral; only the internal `get_ppid()` helper is gated.

`cleanup_old_sessions()` is called from `main.rs` at startup (before `session.load()`). It scans `~/.reel/sessions/` and removes any `.json` files whose last-modified time is older than 7 days. Silent no-op if the directory does not exist yet.

### Data model

```rust
struct State {
    method:    Option<String>,
    url:       Option<String>,
    headers:   HashMap<String, String>,   // keys stored as-is from session/preset files
    body:      Option<String>,
    responses: Vec<ResponseRecord>,   // populated by send/then, never set by the user
}

struct ResponseRecord {
    source:  Option<String>,   // preset file path for `then`, None for plain `send`
    status:  u16,
    headers: HashMap<String, String>,
    body:    String,
}
```

`responses` is part of `State` so it round-trips through the session file automatically. Preset files (used with `load` or `then`) omit this field — `serde` deserialises it as an empty `Vec`. Old session files with a `last` field are silently ignored by serde. The `headers` field also has `#[serde(default)]`, so preset files that omit `headers` entirely are valid and deserialise to an empty map.

### State lifecycle

1. `FileSessionStore::new()` computes the session path (PPID-based); `session.load()` reads it (or returns `State::default()`)
2. `parse_args` processes all CLI arguments left-to-right and returns `(Vec<Command>, GlobalFlags)`; no I/O here
3. `run_commands` executes commands in order, mutating `state`:
   - `load <PATH>` — replaces `state` wholesale from a JSON file
   - `save <PATH>` — writes current in-memory `state` to a preset file (no `responses` field)
   - `reset` — replaces `state` with `State::default()` and calls `session.delete()`
   - `header <KEY:VALUE>` — inserts or overwrites one entry in `state.headers`; key stored as-is (original casing preserved); deduplication on insert is case-insensitive
   - `header-rm <KEY>` — removes one entry case-insensitively; warns if absent
   - `header-rm-all` — removes all entries from `state.headers`
   - `method`, `url`, `body` — overwrite the respective field; `url ""` (empty string) is rejected with an error
   - `send` — clears `state.responses`, executes via `http.execute()`; calls `session.save()` before printing body
   - `then <PATH>` — loads a preset file, carries `state.responses` forward, applies template interpolation against the full response history, executes via `http.execute()` and appends to `state.responses`; aborts the whole chain on failure; errors if preset contains `${{` expressions but there is no previous response
   - `--dry-run` (GlobalFlag) — skips the HTTP call inside `send`/`then`; state mutations still occur and `session.save()` is still called; prints request details (method, URL, headers, body) to stderr
   - `fail` (GlobalFlag) — after all commands complete, exits with code 1 if any `send` or `then` received a 4xx or 5xx response
4. If any mutation occurred without a following `send`/`then`, `session.save()` is called at end
5. `show` and `response` (deferred during parsing) run after all commands, in that order

`session.save()` is called inside the `Send`/`Then` branches **before** any stdout write — broken-pipe safety (e.g. `reel send | head -5`).

`response` reads `state.responses` from the already-loaded in-memory state; it never triggers an additional disk write.

`response [N|all]` in `Full` view uses `response_display_value()` to build the JSON output: the `body` field is embedded as a parsed JSON object when the body is valid JSON, and as a plain string otherwise. The underlying `ResponseRecord.body` is always stored as a raw string.

### Template interpolation (`then`)

Preset files loaded by `then` may contain `${{ expr }}` placeholders in `url`, `body`, and header values. Supported expressions:

| Expression | Resolves to |
|---|---|
| `status` | HTTP status code of the last response (string) |
| `body` | Raw body text of the last response |
| `body.<dot.path>` | Dot-path into the last response body (e.g. `body.access.token`, `body.items.0.id`) |
| `headers.<name>` | Response header value from the last response (e.g. `headers.content-type`) |
| `response[N].status` | HTTP status code of the Nth response (1-based) |
| `response[N].body` | Raw body of the Nth response |
| `response[N].body.<dot.path>` | Dot-path into the Nth response body |
| `response[N].headers.<name>` | Response header value from the Nth response |

The bare `body`/`status`/`headers.*` forms always refer to the **last** response. Use `response[N].*` to reach any earlier response in the chain by 1-based index.

If a placeholder cannot be resolved (missing key, non-JSON body, out-of-range index, unclosed `${{`), the chain aborts immediately with an error. If a preset contains any `${{` but there is no previous response, `then` errors immediately rather than silently treating the placeholder as a literal string.

### Argument parsing

No external parser (no `clap`). `parse_args` is a hand-written `while` loop over `args` that returns `Vec<Command>`. This keeps the UX simple: `reel method GET url https://example.com send` reads naturally left-to-right.

`show` and `response` are deferred — `parse_args` records them in the `Command` stream and `run_commands` sets flags for them, running them after all other commands complete.

The `response` command peeks at the next token(s) to consume an optional `all`/index and/or `body`/`headers` modifier.

`fail`, `--insecure`, and `--dry-run` are pre-scanned before the loop and returned as `GlobalFlags`; they apply globally regardless of position. Their tokens are consumed and not added to `Vec<Command>`.

`parse_args` returns `anyhow::Result<(Vec<Command>, GlobalFlags)>`. `run_commands` returns `anyhow::Result<ParseResult>`. On `Err(_)`, `main` prints the error message to stderr and calls `std::process::exit(1)`. All diagnostic output goes to stderr so it never pollutes piped output. `response headers` writes header data to stdout (it is data, not a diagnostic).

## Dependencies

| Crate | Why |
|---|---|
| `reqwest` (blocking) | HTTP client; blocking avoids async complexity for a CLI |
| `serde` + `serde_json` | State serialization |
| `dirs` | Cross-platform home directory |
| `libc` (Unix only) | PPID lookup via `getppid()` on Linux, macOS, BSD |
| `windows-sys` (Windows only) | PPID lookup via `CreateToolhelp32Snapshot` |

## Build & test

```bash
cargo build              # dev build
cargo build --release    # release build → target/release/reel
cargo fmt --check        # formatting (no changes expected)
cargo clippy             # linter (should produce no warnings)
cargo test               # unit tests for model and template modules
```

Tests live in per-module `tests.rs` files (`model/tests.rs`, `template/tests.rs`, `session/tests.rs`, `cli/tests.rs`). `cli/tests.rs` contains `MockHttp` and `MockSession` test doubles injected via `run_commands` — there are no integration tests with real network calls.

## Extending the tool

Likely next additions and where to put them. Implement only when explicitly requested.

- **Named sessions** (`reel session <name>`) — symlink or alias over `save`/`load`
- **Configurable timeout** (`reel timeout 30`) — currently hardcoded to 30 s; expose as a `State` field
- **Query params** (`reel param key value`) — add `params: HashMap<String,String>` to `State`, pass to `.query()` on the request builder
- **Auth shorthand** (`reel auth bearer <token>`) — sugar over `header Authorization "Bearer <token>"`
- **Verbose mode** — print full request details before sending; flag in `State` or a CLI-only bool
- **Session list/switch** — list `~/.reel/sessions/`, let user pick by number or name

## Constraints to keep

- No async runtime. `reqwest::blocking` is deliberate — a CLI tool doesn't benefit from async.
- No `clap`. The positional key-value syntax is the UX; a flag-based parser would break it.
- Status on stderr, body on stdout. Do not change this — it enables `reel send | jq .`.
- `session.save()` inside `Send`/`Then` command branches must come before any stdout write — broken-pipe safety.
- `parse_args` must stay pure (no I/O). All I/O belongs in `run_commands` via the injected traits.
