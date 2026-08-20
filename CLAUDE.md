# reel — developer notes for Claude

## Project overview

`reel` is a stateful CLI HTTP client written in Rust. It behaves like `curl` but persists request state (method, URL, headers, body) across invocations within the same terminal session.

## Architecture

Modules under `src/`:

| File | Responsibility |
|---|---|
| `main.rs` | Entry point — wires modules together |
| `model/mod.rs` | `Headers`, `Request`, `ResponseRecord`, `State` structs (pure data, no I/O); `status_reason` lookup table |
| `model/cookies.rs` | `Cookie` struct, `Set-Cookie` parsing, domain/path matching, jar update — pure functions, no I/O |
| `session/mod.rs` | `SessionStore` + `PresetStore` traits with `FileSessionStore`/`FilePresetStore` impls; `cleanup_old_sessions` free function |
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

**`http::HttpClient`** — abstracts HTTP execution. `run_commands` receives `&dyn HttpClient`, making it testable without real network calls. It takes only the `Request` — the HTTP layer never sees session history. (`status_reason(code)`, used by `cli/display.rs` to format status lines, is a pure lookup table in `model` — the display layer does not depend on the HTTP module.)

```rust
pub trait HttpClient {
    fn execute(&self, request: &Request, source: Option<&str>) -> Result<ResponseRecord>;
}
```

**`session::SessionStore`** — abstracts session persistence. `run_commands` receives `&dyn SessionStore`. `save` returns `Result` (I/O failure aborts the command chain rather than panicking).

```rust
pub trait SessionStore {
    fn load(&self) -> State;
    fn save(&self, state: &State) -> Result<()>;
    fn delete(&self);
    fn path(&self) -> &Path;
}
```

**`session::PresetStore`** — abstracts preset-file access (`load`/`save`/`then`). Separate from `SessionStore` because presets live at caller-given paths while sessions live at keyed paths the store owns. `run_commands` receives `&dyn PresetStore`, so the preset command paths are testable without disk I/O.

```rust
pub trait PresetStore {
    fn load(&self, path: &Path) -> Result<State>;
    fn save(&self, request: &Request, path: &Path) -> Result<()>;
}
```

`main.rs` constructs the concrete implementations (`ReqwestClient`, `FileSessionStore`, `FilePresetStore`) and passes them in. `FileSessionStore::new()` returns `Result` — it fails (graceful stderr + exit 1, no panic) only when the home directory cannot be determined.

### Session identification

The session key is, in order of precedence:

1. **`REEL_SESSION` env var** — a named session, stored as `~/.reel/sessions/named-<name>.json`. Valid names are 1–64 chars from `[A-Za-z0-9._-]`; an invalid value prints a warning and falls back to the PPID. The `named-` prefix guarantees named files never collide with PID files.
2. **Parent shell PID** — stored as `~/.reel/sessions/<ppid>.json`. The PPID is read once via `OnceLock<u32>` and cached for the process lifetime. Known limitation: under Git Bash/MSYS2 on Windows the shell spawns each native executable via a short-lived intermediate process, so every invocation sees a different PPID and gets a fresh session — `REEL_SESSION` is the workaround (documented in README; the orphan files are removed by the normal dead-PID cleanup).

Platform-specific PPID lookup is gated with `#[cfg(...)]` inside `get_ppid()` in `session/mod.rs`:

| Platform | Mechanism |
|---|---|
| Unix (Linux, macOS, BSD, …) | `libc::getppid()` |
| Windows | `CreateToolhelp32Snapshot` + `Process32FirstW` from `windows-sys` |
| other | Falls back to `0` with a warning (all invocations share one session) |

The `SessionStore` trait itself is platform-neutral; only the internal helpers (`get_ppid()`, `process_start_time()`, `process_alive()`) are gated.

**PID-reuse protection (ownership stamp).** PID-keyed session files carry an `owner_start_time` field: the parent process's start time (Linux: field 22 of `/proc/<pid>/stat`; Windows: `GetProcessTimes` creation `FILETIME`; other platforms: absent). On `load`, a stamp that differs from the current parent's start time means the PID was recycled by a new shell — the file is deleted and the session starts fresh, silently. The check is skipped when either side is unknown (named sessions, unstamped files from older versions, platforms without start times). The stamp lives in the `SessionFile`/`SessionFileRef` wrapper structs in `session/mod.rs`, **not** in `State` — the data model stays free of persistence metadata, and presets are unaffected (unknown fields are ignored on deserialization).

**Atomic, private writes.** `save` serializes to `<path>.<pid>.tmp` and renames over the target, so an interrupted save can never corrupt the session file. On Unix the temp file is created with mode `0600` and the sessions directory with `0700` (session files can contain credentials); on Windows, default ACLs apply. `FilePresetStore::save` writes the same way (presets can carry `Authorization` headers) — new preset files are `0600`, but overwriting keeps the permissions the user gave the existing file.

**Cleanup.** `cleanup_old_sessions(current_session)` is called from `main.rs` at startup — after the store is constructed (so the current invocation's own session file is known and always exempt from cleanup, no matter its age), before `session.load()`. For each `<pid>.json` file it checks whether the owning process is still alive (Unix: `kill(pid, 0)`, treating `EPERM` as alive; Windows: `OpenProcess` + `GetExitCodeProcess == STILL_ACTIVE`, treating `ERROR_ACCESS_DENIED` as alive): dead → removed immediately, alive → kept regardless of age (a week-idle tmux shell keeps its session). Files whose owner cannot be verified — `named-*.json`, the `0.json` fallback, platforms without a liveness check — and leftover `*.tmp` files fall back to the age rule: removed when last modified more than 7 days ago (`MAX_UNVERIFIED_AGE`). `load` refreshes the file's mtime (best-effort), so "age" means time since last *use*, not last save — a named session used daily for reads only never looks abandoned. Silent no-op if the directory does not exist yet.

### Data model

```rust
struct Headers(HashMap<String, String>);  // newtype: case-insensitive get/insert/remove,
                                          // original casing preserved; #[serde(transparent)]

struct Request {                   // the request being built — and, unchanged in shape,
    method:  Option<String>,       // the history snapshot of a request as actually sent
    url:     Option<String>,       // (after template interpolation)
    headers: Headers,
    body:    Option<String>,
    body_file: Option<String>,     // body sourced from a file, read at send time;
                                   // mutually exclusive with `body`
    body_merge: Option<String>,    // RFC 7386 JSON Merge Patch overlaid on the body
                                   // at send time; orthogonal to both body sources
    timeout_secs: Option<u64>,     // None = default (30 s), Some(0) = no timeout
}

struct State {
    request:   Request,            // #[serde(flatten)] — session files keep their flat shape
    requests:  Vec<Request>,       // populated by send/then, never set by the user
    responses: Vec<ResponseRecord>,// populated by send/then, never set by the user
    cookies:   Vec<Cookie>,        // cookie jar — populated from Set-Cookie responses
    vars:      HashMap<String, String>, // session variables — set by the user via `var`
}

struct ResponseRecord {
    source:  Option<String>,   // preset file path for `then`, None for plain `send`
    status:  u16,
    headers: Headers,
    body:    String,
    set_cookies: Vec<String>,  // raw Set-Cookie values (can't be ", "-joined into headers)
    elapsed_ms: u64,           // request duration; 0 in records from older versions
}

struct Cookie {                // model/cookies.rs — simplified RFC 6265
    name:      String,
    value:     String,
    domain:    String,         // lowercase; request host unless a Domain attribute widened it
    path:      String,
    secure:    bool,           // only sent over https
    host_only: bool,           // no Domain attribute: exact-host match only
}
```

All case-insensitive header semantics (lookup, insert dedup, removal) live in `Headers` — callers never scan for matching keys themselves. `requests` and `responses` are part of `State` so they round-trip through the session file automatically. Each `send`/`then` pushes one `Request` snapshot and one `ResponseRecord`, keeping the two vectors index-aligned (`request[N]` is the request that produced `response[N]`). The snapshot is captured **after** template interpolation, so `request.*` reflects what was actually sent. Preset files (used with `load` or `then`) omit both fields — `serde` deserialises them as empty `Vec`s. Old session files with a `last` field are silently ignored by serde. The `headers` field has `#[serde(default)]` and is skipped when empty, so preset files that omit `headers` entirely are valid and deserialise to an empty map. `body` (and `body_merge`) is written to session/preset files as a native JSON value only when that representation reloads to the exact original string (`body_serde::round_trips` in `model/mod.rs`); anything lossy — pretty-printed JSON, `1e3`-style numbers, JSON string/null literals — is stored as a plain string, so body bytes always round-trip unchanged. Key order is not lossy: `serde_json` is built with `preserve_order`, so objects keep the order they were written in, everywhere reel parses and re-emits JSON.

### State lifecycle

1. `FileSessionStore::new()` computes the session path (`REEL_SESSION` name or PPID) and, for PID-keyed sessions, the parent's start time; `session.load()` reads the file (or returns `State::default()`; a stale ownership stamp also yields a fresh default)
2. `parse_args` processes all CLI arguments left-to-right and returns `(Vec<Command>, GlobalFlags)`; no I/O here
3. `run_commands` executes commands in order, mutating `state`:
   - `load <PATH>` — replaces `state` wholesale from a JSON file (then `rebase_preset_paths`), except the cookie jar and session variables: the session's cookies and vars survive `load` (like they survive `send`) unless the loaded file itself carries a `cookies`/`vars` field
   - `save <PATH>` — writes the current in-memory `state.request` to a preset file (never `requests`/`responses`); a write failure aborts the chain with an error
   - `reset` — replaces `state` with `State::default()` and calls `session.delete()`
   - `header <KEY:VALUE>` — `Headers::insert`: key stored as-is (original casing preserved); deduplication on insert is case-insensitive
   - `header-rm <KEY>` — `Headers::remove` (case-insensitive); warns if absent
   - `header-rm-all` — removes all entries from `state.request.headers`
   - `var <NAME> <VALUE>` — sets `state.vars[NAME]` (overwrite on repeat; names are case-sensitive). Names are validated in the parser: 1+ chars from `[A-Za-z0-9_-]`, so they stay usable inside a `${{ var.<name> }}` placeholder. Values are stored literally — like every other CLI value, they are never interpolated at set time (interpolation happens at send time, on a copy)
   - `var-rm <NAME>` — removes a session variable; warns if absent
   - `method`, `url`, `body` — overwrite the respective `state.request` field; `url ""` (empty string) is rejected with an error. `method` uppercases only the standard methods (`normalize_method` in `cli/parser.rs`) — a custom method keeps its casing (RFC 9110: methods are case-sensitive). `body` values are resolved by `resolve_body` in `cli/runner.rs` (not in the parser, which stays I/O-free): `-` reads stdin, `@path` reads a file verbatim, `@@...` escapes a literal leading `@`. Setting `body` clears `body_file`
   - `body-file <PATH>` — stores the *path* in `state.request.body_file` (clearing `body`); the file is read on every send by `prepare_request`, so an edited payload takes effect without re-running the command. A path that does not exist yet is only a warning — the binding is deliberately late. Contrast `body @path`, which reads the file once, in `resolve_body`
   - `body-merge <JSON>` — stores an RFC 7386 JSON Merge Patch in `state.request.body_merge`, applied to the body by `prepare_request` at send time. Values resolve through `resolve_body` like `body` (`@path`, `-`), and the patch is validated as JSON when the command runs, so a typo fails there rather than at the next send. Deliberately does *not* clear `body`/`body_file` — it modifies whichever body source is in force
   - `body-rm` — clears `state.request.body`, `body_file` *and* `body_merge`; warns if none was set
   - `cookie-rm <NAME>` — removes every jar cookie with that name (all domains/paths); warns if none matched
   - `timeout <SECONDS>` — sets `state.request.timeout_secs`; `0` means "no timeout". The HTTP layer applies it per request (`DEFAULT_TIMEOUT_SECS` = 30 when unset); the client itself is built with no timeout so `timeout 0` really disables it
   - `send` — interpolates the request via `prepare_request` (see below), executes via `http.execute()` and replaces the history: `state.requests`/`state.responses` are cleared only once an attempt actually produced a response, then the new snapshot is pushed — a failed send leaves the previous history untouched (in memory and on disk); calls `session.save()` before printing body
   - `curl` — prints the current request as an equivalent curl command (`format_curl` in `cli/display.rs`; goes through `prepare_request` and `with_session_cookies`, so templates are resolved and the jar is reflected — an unresolvable placeholder makes it fail like a send would). Output on stdout — it is data, not a diagnostic. Not a mutation; does not touch the HTTP layer
   - `expect <CONDITION>` — asserts against the session history via `template::check_condition` (which reuses `eval_expr`, so any template expression works: `status`, `body.<path>`, `headers.<name>`, `response[N].*`, `request.*`, `env.*`, `var.*`). Forms: bare `<expr>` (must resolve), `<expr> == <v>`, `<expr> != <v>`, `<expr> contains <v>`. Failure aborts the chain with exit code 1; success prints `expect ok: ...` to stderr. Skipped under `--dry-run`
   - `then <PATH>` — replaces `state.request` from a preset file **verbatim, placeholders intact** (history stays in place), rebases the preset's relative file paths (`rebase_preset_paths`), then sends it through the same path as `send`. Semantically `then <PATH>` is `load <PATH> send` minus the history clear, which is why interpolation no longer lives here: it happens for every send, in `prepare_request`. The only thing `then` adds is the `source` label, used for the `error in '<path>': ...` message and `ResponseRecord.source`.

   **`prepare_request` (`cli/runner.rs`).** Every send builds the wire request from `state.request` on a *copy*: a `body_file` path is read from disk into `body`, then `apply_interpolation` resolves every `${{ }}` placeholder against the history, the environment, and `state.vars` (in `url`, `body`, `body_merge` and header values), and finally `apply_body_merge` overlays `body_merge` on the body — both sides already interpolated, so a payload file and its overlay may each carry placeholders. A patch with no body, or on a body that is not valid JSON, is an error rather than a silent passthrough; a merged body is re-serialised compactly, so a payload file's own formatting survives only when no patch is set. The stored request keeps its template text, so a placeholder resolves afresh on each send and a changed `var`/env var takes effect without reloading the preset. It runs once per `execute_and_record` call, *outside* the attempt loop — a retry must resend the same bytes, so `${{ uuid() }}` idempotency keys stay stable across attempts. Failure aborts before the HTTP layer is touched. `--dry-run` and `curl` go through it too, so what they print is what would be sent.

   The shared execution path of `send`/`then` (execute → record history → `session.save()` → print → honour `fail`) lives in the `execute_and_record` helper in `cli/runner.rs`. The "body set but Content-Type missing" warning is also emitted there, not in the HTTP layer.

   **Cookie jar.** `execute_and_record` sends the request through `with_session_cookies`: matching cookies from `state.cookies` are assembled into a `Cookie` header (a user-set `Cookie` header always wins; the history snapshot records the header as actually sent). After the response, raw `Set-Cookie` values from `ResponseRecord.set_cookies` are parsed into the jar (`model/cookies.rs`): same `(name, domain, path)` replaces, `Max-Age <= 0` deletes, a `Domain` attribute that is not a suffix of the request host is rejected. Matching is simplified RFC 6265 (domain + path + `Secure`); `Expires`/`Max-Age` lifetimes are not tracked — cookies live until `reset`, `cookie-rm`, or deletion by the server. The jar survives `send` (which only clears `requests`/`responses`) and is persisted in the session file; presets never contain it. Every jar update marks the session modified, so cookies picked up by non-final retry/poll attempts survive (via the best-effort end-of-run save) even when the chain later aborts. Cookie handling is skipped entirely when the URL cannot be parsed (`url_parts` in `cli/runner.rs`).
   - `--dry-run` (GlobalFlag) — `send`/`then` print the request details (method, URL, headers, body) to stderr and skip the HTTP call, history recording, and their own `session.save()`. Mutations made by other commands (`method`, `url`, `header`, `load`, …) still occur and are persisted by the end-of-run save; a dry-run `then` interpolates the preset into `state.request` in memory but does not itself mark the session as modified
   - `fail` (GlobalFlag) — after all commands complete, exits with code 1 if any `send` or `then` received a 4xx or 5xx response. The verdict is deferred (`ChainState.http_failure` in `cli/runner.rs`): a 4xx/5xx does **not** abort the chain, so later `then` steps still run and the session is saved before the failing exit
   - `--follow` (GlobalFlag) — opt-in redirect following (curl `-L` semantics), implemented as a hop loop in `execute_following` in `cli/runner.rs` (never via reqwest's redirect policy, which stays `none`). Up to `MAX_REDIRECTS` (10) hops; exceeding the cap is a `PermanentError` (no retry burn). Per hop: cookies are absorbed into the jar, `303` (and `301`/`302` after `POST`) switches to `GET` and drops the body + its `Content-Type`, a cross-host hop strips the user-set `Authorization`/`Cookie` headers, and the jar-injected `Cookie` header is re-derived for the new URL. Only the final hop's request/response pair is recorded in history; each hop prints a `following redirect -> <url>` note on stderr
   - `--retry <N>` / `--until <COND>` / `--delay <S>` (GlobalFlags) — the attempt loop lives in `execute_and_record`. `--retry` grants N extra attempts on transient failure (network error or transient 5xx — 4xx and the permanent 501/505/506/510 are never retried; see `transient_5xx`). Errors the HTTP layer marks with the `PermanentError` type (bad URL, method, or header — deterministic builder failures) abort immediately regardless of budget. `--until` polls until the condition (evaluated via `template::check_condition` against the history *plus the candidate request and response*, so indices stay aligned and `request.*` sees the in-flight request) passes; budget is `retry + 1`, or 10 when `--retry` is absent (`retry` is `Option<u32>` — an explicit `--retry 0` caps the poll at a single attempt, unlike an absent flag); on exhaustion the last response is still recorded, saved, and printed before the chain aborts. Only the final attempt enters the history. These flags take values consumed in the parser loop (still position-independent — flags apply only after parsing completes)
   - All boolean flags accept both spellings (`fail`/`--fail`, `insecure`/`--insecure`, `dry-run`/`--dry-run`, `follow`/`--follow`) and the value-taking ones accept the bare form too (`retry`/`until`/`delay`)
4. If any mutation occurred without a following `send`/`then`, `session.save()` is called at end — also (best-effort, errors ignored so they cannot mask the chain error) when the chain aborts, so `reel url X send` does not lose the `url` mutation on a network failure
5. `show` and `response` (deferred during parsing) run after all commands, in that order

`session.save()` is called inside the `Send`/`Then` branches **before** any stdout write — broken-pipe safety (e.g. `reel send | head -5`).

`response` reads `state.responses` from the already-loaded in-memory state; it never triggers an additional disk write.

`response [N|all]` in `Full` view uses `response_display_value()` to build the JSON output: the `body` field is embedded as a parsed JSON object when the body is valid JSON, and as a plain string otherwise. The underlying `ResponseRecord.body` is always stored as a raw string.

### Template interpolation

`url`, `body`, and header values may contain `${{ expr }}` placeholders, wherever they came from — a preset file or a plain `reel url ...`. They are resolved at send time by `prepare_request`, never when the value is set (CLI values are stored literally). Supported expressions:

| Expression | Resolves to |
|---|---|
| `status` | HTTP status code of the last response (string) |
| `elapsed` | Duration of the last request in milliseconds (also `response[N].elapsed`) |
| `body` | Raw body text of the last response |
| `body.<dot.path>` | Dot-path into the last response body (e.g. `body.access.token`, `body.items.0.id`) |
| `headers.<name>` | Response header value from the last response (e.g. `headers.content-type`) |
| `response[N].status` | HTTP status code of the Nth response (1-based) |
| `response[N].body` | Raw body of the Nth response |
| `response[N].body.<dot.path>` | Dot-path into the Nth response body |
| `response[N].headers.<name>` | Response header value from the Nth response |
| `request.method` / `request.url` / `request.body` | Field of the last request (as actually sent) |
| `request.body.<dot.path>` | Dot-path into the last request body |
| `request.headers.<name>` | Request header value from the last request (case-insensitive) |
| `request[N].<field>` | Any of the above `request` fields for the Nth request (1-based) |
| `env.<NAME>` | Value of environment variable `NAME` |
| `var.<name>` | Session variable set with `reel var <name> <value>` (case-sensitive) |
| `uuid()` | Random v4 UUID (each placeholder evaluated independently) |
| `now()` / `now(±N)` | Current Unix timestamp in seconds, optionally shifted by N seconds |
| `base64(<arg>)` | Base64 of the argument: a single-quoted literal (`base64('user:pass')`) or a nested expression (`base64(env.CREDS)`), evaluated recursively |
| `file(<arg>)` | Contents of a file; argument as for `base64`. The result is *not* interpolated again. A missing file is `Unresolvable`, so `\| default:` covers it |
| `<expr> \| default: <value>` | Fallback when `<expr>` cannot be resolved (unset env var, missing history/key — errors carrying the `Unresolvable` marker in `template/mod.rs`). Structural errors (unknown function or expression, bad index syntax) propagate even with a default, so a typo cannot silently become the fallback. Split at the first `\|` outside single quotes; handled in `eval_placeholder` (interpolation only — `expect`/`--until` conditions do not support it) |

The bare `body`/`status`/`headers.*` forms always refer to the **last** response; bare `request.*` refers to the **last** request. Use `response[N].*` / `request[N].*` to reach any earlier entry in the chain by 1-based index. Requests and responses share the same 1-based index (`request[N]` is the request that produced `response[N]`). Request records are captured *after* interpolation, so `request.*` reflects the values actually sent.

`env.*` reads the process environment and `var.*` the session's `vars` map; both are independent of history, so a preset that only uses `${{ env.* }}`/`${{ var.* }}` interpolates successfully even with no previous request/response. Like the cookie jar, `vars` survive `send` and `load` and are cleared only by `reset`; `save` never writes them to preset files (it writes only the request).

`$${{ ... }}` is an escape yielding a literal `${{ ... }}` — a request whose own payload is template-shaped (a CI workflow, say) must still go out unchanged. Handled in `interpolate`, and skipped over identically by `map_placeholders`.

**Preset-relative paths.** `body_file` and the quoted argument of `file('...')` may be relative. `rebase_preset_paths` (`cli/runner.rs`) runs once, when `load`/`then` reads the file (covering `url`, `body`, `body_merge` and header values), and rewrites each such path to an absolute one *if* it exists next to the preset; otherwise it is left alone and stays relative to the working directory at send time. Absolutising matters because the rebased path is persisted in the session and read again from a possibly different directory. The placeholder-aware rewrite is `template::rebase_file_literals` (single path: `template::rebase_path`); it only touches `file('<literal>')` — a nested expression like `file(var.p)` is the user's own to resolve.

If a placeholder cannot be resolved (missing key, non-JSON body, out-of-range index, unset environment variable, unclosed `${{`), the chain aborts immediately with an error. Interpolation is always attempted; a placeholder referencing history that does not exist yet (e.g. `${{ body }}` before any `send`) aborts with a "no previous response/request" error rather than being treated as a literal string.

### Argument parsing

No external parser (no `clap`). `parse_args` is a hand-written loop over a small `Tokens` cursor (`next`/`value_for`/`rest`/`advance`) that returns `Vec<Command>`; `value_for` produces the uniform `'<cmd>' requires ...` errors. This keeps the UX simple: `reel method GET url https://example.com send` reads naturally left-to-right. The chained command-stream grammar fundamentally doesn't fit clap's one-subcommand-per-invocation model — this was re-evaluated and deliberately re-affirmed.

`show` and `response` are deferred — `parse_args` records them in the `Command` stream and `run_commands` sets flags for them, running them after all other commands complete.

Verb shortcuts (`get`/`post`/`put`/`patch`/`delete`/`head`/`options <URL>`) expand at parse time to `Method` + `Url`, plus one implied `Send` inserted after the loop — before the first `Expect` that appears after the verb, otherwise at the end — so modifiers written after the verb still apply to the request, while an `Expect` written before the verb keeps asserting on the prior session history. The implied `Send` is skipped when the command stream already contains an explicit `Send`/`Then`. A second verb shortcut in one invocation is a parse error (it would silently overwrite the first request without sending it).

The `response` command peeks at the next token(s) to consume an optional `all`/index and/or `body`/`headers` modifier.

`fail`, `--insecure`/`insecure`, `--dry-run`/`dry-run`, `help`/`-h`/`--help`, and `-V`/`--version` are handled by their own match arms in the loop and returned as `GlobalFlags`; they apply globally regardless of position **among commands**, but a token consumed as a command's value (`header X-Mode insecure`, `body fail`) is never treated as a flag — critical for `insecure`, which disables TLS verification. `main` prints usage/version and exits before any session I/O when `help`/`version` is set.

`parse_args` returns `anyhow::Result<(Vec<Command>, GlobalFlags)>`. `run_commands` returns `anyhow::Result<ParseResult>`. On `Err(_)`, `main` prints the error message to stderr and calls `std::process::exit(1)`. All diagnostic output goes to stderr so it never pollutes piped output. `response headers` writes header data to stdout (it is data, not a diagnostic).

## Dependencies

| Crate | Why |
|---|---|
| `reqwest` (blocking) | HTTP client; blocking avoids async complexity for a CLI |
| `serde` + `serde_json` | State serialization; `preserve_order` keeps JSON object keys in their original order instead of sorting them alphabetically |
| `dirs` | Cross-platform home directory |
| `url` | Host/path extraction from the request URL for cookie scoping (already in the tree via reqwest) |
| `uuid` (v4) | `${{ uuid() }}` template function |
| `libc` (Unix only) | PPID lookup via `getppid()`; process liveness via `kill(pid, 0)` |
| `windows-sys` (Windows only) | PPID lookup via `CreateToolhelp32Snapshot`; process start time / liveness via `OpenProcess`, `GetProcessTimes`, `GetExitCodeProcess` |

## Build & test

```bash
cargo build              # dev build
cargo build --release    # release build → target/release/reel
cargo fmt --check        # formatting (no changes expected)
cargo clippy             # linter (should produce no warnings)
cargo test               # unit tests for model and template modules
```

Tests live in per-module `tests.rs` files (`model/tests.rs`, `template/tests.rs`, `session/tests.rs`, `cli/tests.rs`). `cli/tests.rs` contains `MockHttp`, `MockSession`, and `MockPresets` test doubles injected via `run_commands` — the `load`/`save`/`then` command paths are covered through the in-memory `MockPresets`, and there are no integration tests with real network calls or disk I/O.

## Extending the tool

Likely next additions and where to put them. Implement only when explicitly requested.

- **Named sessions** (`reel session <name>`) — symlink or alias over `save`/`load`
- **Query params** (`reel param key value`) — add `params: HashMap<String,String>` to `Request`, pass to `.query()` on the request builder
- **Auth shorthand** (`reel auth bearer <token>`) — sugar over `header Authorization "Bearer <token>"`
- **Verbose mode** — print full request details before sending; flag in `State` or a CLI-only bool
- **Session list/switch** — list `~/.reel/sessions/`, let user pick by number or name

## Constraints to keep

- No async runtime. `reqwest::blocking` is deliberate — a CLI tool doesn't benefit from async.
- No `clap`. The positional key-value syntax is the UX; a flag-based parser would break it.
- Status on stderr, body on stdout. Do not change this — it enables `reel send | jq .`.
- Body output goes through `print_body` in `cli/display.rs`: pretty-printed JSON on a TTY, raw bytes when piped. The piped path must stay byte-exact (`response body` adds no trailing newline; `send`/`then` pad a missing final newline as before). Exception by necessity: bodies that are not valid UTF-8 cannot round-trip through the `String` model — `read_body` in `http.rs` replaces invalid bytes and warns on stderr; a declared non-UTF-8 charset is transcoded by reqwest.
- No automatic redirect following at the HTTP layer (`redirect(Policy::none())` in `http.rs`) — curl parity; 3xx responses surface to the user, so the cookie jar sees every hop and `Cookie`/`Authorization` can never ride a cross-scheme redirect. Do not re-enable reqwest's default policy. The opt-in `--follow` flag loops in `cli/runner.rs` (`execute_following`) instead, precisely so every hop passes through the jar and the cross-host credential-stripping rules.
- `session.save()` inside the shared `send`/`then` execution path (`execute_and_record` in `cli/runner.rs`) must come before any stdout write — broken-pipe safety. `main` additionally restores `SIGPIPE` to `SIG_DFL` on Unix so `reel send | head` exits quietly instead of panicking.
- Global flags are set only from command-position match arms in `parse_args` — never from a positional pre-scan, which would let a header/body *value* like `insecure` disable TLS verification.
- `parse_args` must stay pure (no I/O). All I/O belongs in `run_commands` via the injected traits.
- Interpolation happens on a copy in `prepare_request`, never in place on `state.request` — the session must keep the template text, or a placeholder would resolve exactly once and then be burnt into the session file.
