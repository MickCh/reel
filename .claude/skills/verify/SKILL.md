---
name: verify
description: Build and drive reel end-to-end against a local echo server to verify changes at the CLI surface.
---

# Verifying reel

## Build & handle

```bash
cargo build            # binary at target/debug/reel
```

## Isolation gotchas

- Session state is keyed by **parent shell PID** and stored under `~/.reel/sessions/<ppid>.json`.
  - Set `HOME=$(mktemp -d)` to avoid touching the real `~/.reel`.
  - Run all `reel` invocations of one scenario **inside a single Bash call** — each Bash tool call is a new shell, so a new PPID means a new session.
- Status/diagnostics go to stderr, body to stdout — capture both (`2>&1`, or split with `2>err.txt`).

## Local echo server

Start a tiny Python HTTP server that echoes method/path/headers/body as JSON
(plus routes like `/missing` → 404, `/teapot` → 418) on `127.0.0.1:8642`,
run it in the background, `pkill -f echo_server.py` when done. reqwest sends
header names lowercased — check echoed request headers in lowercase.

## Flows worth driving

- `reel method GET url http://127.0.0.1:8642/x send` then `reel show` (state persists across invocations)
- Header case-insensitivity: `header X-Foo:bar` then `header x-foo:baz` → one entry; `header-rm X-FOO`
- `then` chain with `${{ body.* }}`, `${{ status }}`, `${{ request[N].* }}`, `${{ env.* }}` placeholders
- `save`/`load` preset round-trip; preset file must stay flat JSON without `requests`/`responses`
- `fail` flag on a 404 → exit 1; `--dry-run` → no HTTP call
- Broken pipe: `reel send | head -c 40` must not panic (session saved before stdout write)
- Legacy session file with a `last` field must still load
