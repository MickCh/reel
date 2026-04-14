# req — developer notes for Claude

## Project overview

`req` is a stateful CLI HTTP client written in Rust. It behaves like `curl` but persists request state (method, URL, headers, body) across invocations within the same terminal session.

## Architecture

Single-file application: `src/main.rs`. No modules — the tool is intentionally small.

### Session identification

Uses the parent shell PID (`PPid` from `/proc/self/status`) as the session key. State is stored in `~/.req/sessions/<ppid>.json`. This is Linux-specific; porting to macOS/Windows would require a different PPID lookup.

### State lifecycle

1. Load `~/.req/sessions/<ppid>.json` (or start with `State::default()`)
2. Process all CLI arguments left-to-right, mutating `state` in memory:
   - `file <PATH>` replaces `state` wholesale from a JSON file
   - `save <PATH>` writes the current in-memory `state` to a file (does not affect the session)
   - `reset` replaces `state` with `State::default()` mid-parse
   - All other commands update individual fields
3. If any mutation occurred, persist `state` to `~/.req/sessions/<ppid>.json`
4. Run `show` and/or `exec` after all mutations are applied

`save` writes the in-memory state at the point it appears in the argument list, but because `show`/`exec` run last, combining them is always consistent: `req file a.json save b.json exec` loads, saves a copy, then executes.

### Argument parsing

No external parser (no `clap`). A hand-written `while` loop over `args` processes token pairs. This keeps the UX simple: `req method GET url https://example.com exec` reads naturally left-to-right.

Order within a single invocation matters for `reset` (it clears state mid-parse), but `show` and `exec` always run after all mutations are applied.

## Dependencies

| Crate | Why |
|---|---|
| `reqwest` (blocking) | HTTP client; blocking feature avoids async complexity for a CLI |
| `serde` + `serde_json` | State serialization |
| `dirs` | Cross-platform home directory |

## Build & test

```bash
cargo build              # dev build
cargo build --release    # release build → target/release/req
cargo test               # (no tests yet)
```

Quick smoke test:
```bash
./target/debug/req method GET url https://httpbin.org/get exec
./target/debug/req show
./target/debug/req reset
```

## Extending the tool

Likely next additions and where to put them:

- **Named sessions** (`req session <name>`) — symlink or alias over `save`/`file`
- **Query params** (`req param key value`) — add `params: HashMap<String,String>` to `State`, pass to `.query()` on the request builder
- **Auth shorthand** (`req auth bearer <token>`) — sugar over `header Authorization "Bearer <token>"`
- **Response headers** — `resp.headers()` before consuming `.text()`
- **Timeout** (`req timeout 30`) — `client::Builder::timeout()`
- **Verbose mode** — print request details before sending; flag in `State` or a CLI-only flag
- **Session list/switch** — list `~/.req/sessions/`, let user pick by number or name

## Constraints to keep

- No async runtime. `reqwest::blocking` is deliberate — a CLI tool doesn't benefit from async.
- No `clap`. The positional key-value syntax is the UX; a flag-based parser would break it.
- Status on stderr, body on stdout. Do not change this — it enables `req exec | jq .`.
