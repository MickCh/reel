# reel — developer notes for Claude

## Project overview

`reel` is a stateful CLI HTTP client written in Rust. It behaves like `curl` but persists request state (method, URL, headers, body) across invocations within the same terminal session.

## Architecture

Five modules under `src/`:

| File | Responsibility |
|---|---|
| `main.rs` | Entry point — wires modules together |
| `model.rs` | `State`, `LastResponse` structs (pure data, no I/O) |
| `session.rs` | Session persistence (`get_ppid`, `session_path`, `load_state`, `save_state`) |
| `template.rs` | Template interpolation engine (`${{ expr }}`) |
| `http.rs` | HTTP execution (`execute`) |
| `cli.rs` | Argument parser, display functions (`show_state`, `show_last`, `print_usage`) |

Dependency direction: `cli` → `http`/`template`/`session` → `model`. No module depends on a layer above it.

### Session identification

Uses the parent shell PID (`PPid` from `/proc/self/status`) as the session key. State is stored in `~/.reel/sessions/<ppid>.json`. This is Linux-specific; porting to macOS/Windows would require a different PPID lookup.

### Data model

```rust
struct State {
    method:  Option<String>,
    url:     Option<String>,
    headers: HashMap<String, String>,
    body:    Option<String>,
    last:    Option<LastResponse>,   // populated by exec, never set by the user
}

struct LastResponse {
    status:  u16,
    headers: HashMap<String, String>,
    body:    String,
}
```

`last` is part of `State` so it round-trips through the session file automatically. Preset files (used with `load`) may omit the `last` field — `serde` deserialises it as `None`.

### State lifecycle

1. Load `~/.reel/sessions/<ppid>.json` (or start with `State::default()`)
2. Process all CLI arguments left-to-right, mutating `state` in memory:
   - `load <PATH>` — replaces `state` wholesale from a JSON file
   - `save <PATH>` — writes current in-memory `state` to a file (does not affect the session file)
   - `reset` — replaces `state` with `State::default()` mid-parse
   - `header <KEY:VALUE>` — inserts or overwrites one entry in `state.headers`
   - `header-rm <KEY>` — removes one entry from `state.headers`; warns if the key is absent
   - `method`, `url`, `body` — overwrite the respective field
   - `send` — executes immediately inline (not deferred); calls `save_state` before printing body
   - `then <PATH>` — loads a preset file, applies template interpolation from `state.last`, then executes immediately inline; aborts the whole chain on failure
3. If any mutation occurred without a following `send`/`then`, persist `state` at end of loop
4. Run `show` and/or `last` — always after the main loop, in that order

`send` calls `save_state` itself (updating `state.last`) **before** printing the body, so a broken pipe (e.g. `reel send | head -5`) never prevents the response from being persisted.

`last` reads `state.last` from the already-loaded in-memory state; it never triggers an additional disk write.

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

No external parser (no `clap`). A hand-written `while` loop over `args` processes token pairs. This keeps the UX simple: `reel method GET url https://example.com exec` reads naturally left-to-right.

`send` and `then` execute inline during the loop (not deferred). `show` and `last` are deferred and run once after the loop.

The `last` command peeks at the next token and consumes it only if it is a known subcommand (`body`, `headers`). Any other token is left in place for the main loop to process.

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
cargo test               # (no tests yet)
```

Quick smoke test (all in one shell invocation to share the same PPID session):

```bash
./target/debug/reel method GET url https://httpbin.org/get send > /dev/null \
  && ./target/debug/reel last headers \
  && ./target/debug/reel last body | jq .url \
  && ./target/debug/reel header "X-Foo: bar" header-rm X-Foo show \
  && ./target/debug/reel reset
```

Chaining smoke test:

```bash
# step1.json: GET https://httpbin.org/get
# step2.json: GET https://httpbin.org/anything, header X-Token: ${{ body.some.field }}
./target/debug/reel load step1.json send then step2.json last body | jq .headers
```

## Extending the tool

Likely next additions and where to put them:

- **Named sessions** (`reel session <name>`) — symlink or alias over `save`/`load`
- **Conditional chaining** — stop chain if status ≥ 400 (currently any non-network failure aborts)
- **`last` to file** (`reel last body > out.json`) — already works via stdout; no code change needed
- **Query params** (`reel param key value`) — add `params: HashMap<String,String>` to `State`, pass to `.query()` on the request builder
- **Auth shorthand** (`reel auth bearer <token>`) — sugar over `header Authorization "Bearer <token>"`
- **Timeout** (`reel timeout 30`) — `client::Builder::timeout()`
- **Verbose mode** — print full request details before sending; flag in `State` or a CLI-only bool
- **Session list/switch** — list `~/.reel/sessions/`, let user pick by number or name

## Constraints to keep

- No async runtime. `reqwest::blocking` is deliberate — a CLI tool doesn't benefit from async.
- No `clap`. The positional key-value syntax is the UX; a flag-based parser would break it.
- Status on stderr, body on stdout. Do not change this — it enables `reel send | jq .`.
- `save_state` inside `send` (`http::execute`) must come before any stdout write — broken-pipe safety.
