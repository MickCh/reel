# reel — developer notes for Claude

## Project overview

`reel` is a stateful CLI HTTP client written in Rust. It behaves like `curl` but persists request state (method, URL, headers, body) across invocations within the same terminal session.

## Architecture

Five modules under `src/`:

| File | Responsibility |
|---|---|
| `main.rs` | Entry point — wires modules together |
| `model.rs` | `State`, `LastResponse` structs (pure data, no I/O) |
| `session.rs` | Session persistence (`get_ppid`, `session_path`, `load_state`, `save_state`, `delete_session`) |
| `template.rs` | Template interpolation engine (`${{ expr }}`) |
| `http.rs` | HTTP execution (`build_client`, `execute`) |
| `cli.rs` | Argument parser, display functions (`show_state`, `show_response`, `print_usage`) |

Dependency direction: `cli` → `http`/`template`/`session` → `model`. No module depends on a layer above it.

### Session identification

Uses the parent shell PID (`PPid` from `/proc/self/status`) as the session key. State is stored in `~/.reel/sessions/<ppid>.json`. This is Linux-specific; porting to macOS/Windows would require a different PPID lookup.

### Data model

```rust
struct State {
    method:    Option<String>,
    url:       Option<String>,
    headers:   HashMap<String, String>,
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

`responses` is part of `State` so it round-trips through the session file automatically. Preset files (used with `load` or `then`) omit this field — `serde` deserialises it as an empty `Vec`. Old session files with a `last` field are silently ignored by serde.

### State lifecycle

1. Load `~/.reel/sessions/<ppid>.json` (or start with `State::default()`)
2. Process all CLI arguments left-to-right, mutating `state` in memory:
   - `load <PATH>` — replaces `state` wholesale from a JSON file
   - `save <PATH>` — writes current in-memory `state` to a file (does not affect the session file)
   - `reset` — replaces `state` with `State::default()` and deletes the session file
   - `header <KEY:VALUE>` — inserts or overwrites one entry in `state.headers`
   - `header-rm <KEY>` — removes one entry from `state.headers`; warns if the key is absent
   - `method`, `url`, `body` — overwrite the respective field
   - `send` — clears `state.responses`, executes immediately inline; calls `save_state` before printing body
   - `then <PATH>` — loads a preset file, carries `state.responses` forward, applies template interpolation from the last response, executes immediately inline and appends to `state.responses`; aborts the whole chain on failure
3. If any mutation occurred without a following `send`/`then`, persist `state` at end of loop
4. Run `show` and/or `response`/`responses` — always after the main loop, in that order

`send` calls `save_state` itself (appending to `state.responses`) **before** printing the body, so a broken pipe (e.g. `reel send | head -5`) never prevents the response from being persisted.

`response` reads `state.responses` from the already-loaded in-memory state; it never triggers an additional disk write.

### Template interpolation (`then`)

Preset files loaded by `then` may contain `${{ expr }}` placeholders in `url`, `body`, and header values. Supported expressions:

| Expression | Resolves to |
|---|---|
| `status` | HTTP status code of the previous response (string) |
| `body` | Raw body text of the previous response |
| `body.<dot.path>` | Dot-path into the JSON body (e.g. `body.access.token`, `body.items.0.id`) |
| `headers.<name>` | Response header value (e.g. `headers.content-type`) |

If a placeholder cannot be resolved (missing key, non-JSON body, unclosed `${{`), the chain aborts immediately with an error.

### Argument parsing

No external parser (no `clap`). A hand-written `while` loop over `args` processes token pairs. This keeps the UX simple: `reel method GET url https://example.com send` reads naturally left-to-right.

`send` and `then` execute inline during the loop (not deferred). `show` and `response`/`responses` are deferred and run once after the loop.

The `response` command peeks at the next token(s) to consume an optional index and/or `body`/`headers` modifier. Any other token is left in place for the main loop to process.

`fail` sets a flag that causes the run to exit with code 1 if any subsequent `send` or `then` receives a 4xx/5xx response. `--insecure` skips TLS certificate verification (pre-scanned before the client is built).

`parse_and_run` returns `Result<ParseResult, ()>`. On `Err(())`, `main` calls `std::process::exit(1)`. All user-visible status messages (reset, save, load) go to stderr so they never pollute piped output.

## Dependencies

| Crate | Why |
|---|---|
| `reqwest` (blocking) | HTTP client; blocking avoids async complexity for a CLI |
| `serde` + `serde_json` | State serialization |
| `dirs` | Cross-platform home directory |

## Build & test

```bash
cargo build              # dev build
cargo build --release    # release build → target/release/reel
cargo clippy             # linter (should produce no warnings)
cargo test               # unit tests for model and template modules
```

Quick smoke test (all in one shell invocation to share the same PPID session):

```bash
./target/debug/reel method GET url https://httpbin.org/get send > /dev/null \
  && ./target/debug/reel response headers \
  && ./target/debug/reel response body | jq .url \
  && ./target/debug/reel header "X-Foo: bar" header-rm X-Foo show \
  && ./target/debug/reel reset
```

Chaining smoke test:

```bash
# step1.json: GET https://httpbin.org/get
# step2.json: GET https://httpbin.org/anything, header X-Token: ${{ body.some.field }}
./target/debug/reel load step1.json send then step2.json response body | jq .headers
```

## Extending the tool

Likely next additions and where to put them:

- **Named sessions** (`reel session <name>`) — symlink or alias over `save`/`load`
- **Configurable timeout** (`reel timeout 30`) — currently hardcoded to 30 s; expose as a `State` field
- **Response to file** (`reel response body > out.json`) — already works via stdout; no code change needed
- **Query params** (`reel param key value`) — add `params: HashMap<String,String>` to `State`, pass to `.query()` on the request builder
- **Auth shorthand** (`reel auth bearer <token>`) — sugar over `header Authorization "Bearer <token>"`
- **Verbose mode** — print full request details before sending; flag in `State` or a CLI-only bool
- **Session list/switch** — list `~/.reel/sessions/`, let user pick by number or name

## Constraints to keep

- No async runtime. `reqwest::blocking` is deliberate — a CLI tool doesn't benefit from async.
- No `clap`. The positional key-value syntax is the UX; a flag-based parser would break it.
- Status on stderr, body on stdout. Do not change this — it enables `reel send | jq .`.
- `save_state` inside `http::execute` must come before any stdout write — broken-pipe safety.
