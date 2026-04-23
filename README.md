# reel

[![CI](https://github.com/MickCh/reel/actions/workflows/ci.yml/badge.svg)](https://github.com/MickCh/reel/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust: 1.95+](https://img.shields.io/badge/rust-1.95%2B-orange.svg)](https://www.rust-lang.org)

A stateful HTTP client for the command line. Like `curl`, but it remembers your settings across invocations — per terminal window.

The name comes from a film reel: each request is a frame, and the session threads them together into a single strip.

## Why not curl or httpie?

`curl` and `httpie` are stateless — every invocation starts from scratch. When iterating on an API (adjusting headers, changing the body, re-sending), you end up repeating most of the command each time or reaching for shell history.

`reel` keeps the request state between calls in the same terminal, so you can set headers once, tweak the URL or body, and keep sending without rebuilding the full command.

## How sessions work

Each terminal window runs its own shell process. `reel` uses the parent shell's PID as a session key, storing state in `~/.reel/sessions/<ppid>.json`. This means:

- Settings persist between invocations **in the same terminal window**
- Different terminal windows have **independent sessions**
- A session disappears naturally when the shell exits

## Platform support

| Platform | Session isolation |
|---|---|
| Linux | per terminal window |
| macOS | per terminal window |
| Windows | per terminal window |
| other Unix (BSD, etc.) | per terminal window |
| other | degraded — all windows share one session |

## Installation

### From GitHub (recommended)

```bash
cargo install --git https://github.com/MickCh/reel.git
```

### From source

```bash
cargo build --release
sudo cp target/release/reel /usr/local/bin/
```

## Commands

Commands can be combined freely in a single invocation.

| Command | Description |
|---|---|
| `method <METHOD>` | Set HTTP method (`GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`, `OPTIONS`, …) |
| `url <URL>` | Set request URL |
| `header <KEY:VALUE>` | Add a header (`KEY:VALUE` or `KEY VALUE`) |
| `header-rm <KEY>` | Remove a header by name |
| `header-rm-all` | Remove all headers |
| `body <BODY>` | Set request body |
| `send` | Send the request using current session state |
| `--dry-run` | Print the request that would be sent, without sending it (position-independent) |
| `then <PATH>` | Load preset file, interpolate `${{ expr }}` from response history, and send |
| `show` | Print the current session state |
| `response [N\|all] [body\|headers]` | Show Nth response (default: last), or all; full JSON, body only, or headers only |
| `reset` | Clear all session state |
| `load <PATH>` | Load state from a JSON file |
| `save <PATH>` | Save current session state to a JSON file |
| `fail` | Exit with code 1 if any `send` or `then` receives a 4xx/5xx response (position-independent) |
| `--insecure` | Skip TLS certificate verification |

## Usage examples

### Build a request step by step

```bash
reel url https://api.example.com/users
reel method POST
reel header "Content-Type: application/json"
reel header "Authorization: Bearer mytoken"
reel body '{"name": "Alice"}'
reel show      # inspect before sending
reel send
```

### One-liner

```bash
reel method GET url https://httpbin.org/get send
```

### Reuse settings across calls

```bash
# Set base config once
reel url https://api.example.com/users
reel header "Authorization: Bearer mytoken"

# Subsequent calls reuse the stored state
reel send
reel url https://api.example.com/posts send
```

### Inspect responses

```bash
reel response           # full JSON of the last response
reel response headers   # status code + response headers
reel response body      # raw response body

# pipe-friendly
reel response body | jq .name
reel response body | jq '.users[] | .email'

# access a specific step in a chain (1-based index)
reel response 1 body    # body of the first response
reel response 2         # full JSON of the second response

# see all responses at once
reel response all
reel response all body
reel response all headers
```

`response` reads from the stored session state, so it works even after the
original `send` has finished — no need to re-run the request.

### Save and restore sessions

```bash
# Save current session to a file
reel save staging.json

# Restore it later (in any terminal window)
reel load staging.json send

# Set up, save, and execute in one go
reel method POST url https://api.example.com save prod.json send

# Derive a new preset from an existing one
reel load prod.json url https://staging.example.com save staging.json
```

This is useful for sharing named presets between terminals or keeping configurations for different environments.

### Dry-run: inspect before sending

Use `--dry-run` to print the full request (method, URL, headers, body) to stderr without sending it. Useful before destructive calls.

```bash
reel method DELETE url https://api.example.com/users/42 --dry-run send
reel load prod.json --dry-run send
```

All state mutations still take effect and are saved; only the HTTP call is skipped.

### Chaining requests with `then`

`then` loads a preset file, fills in `${{ expr }}` placeholders using the full response history, and sends the request immediately. This lets you chain dependent calls without scripting.

The first request in a chain is loaded explicitly with `load` and sent with `send`. This is intentional — it lets you inspect or modify the session state before committing to the chain.

```bash
reel load login.json send then dashboard.json
```

Supported expressions in preset files:

| Expression | Resolves to |
|---|---|
| `${{ status }}` | HTTP status code of the **last** response |
| `${{ body }}` | Raw body of the **last** response |
| `${{ body.some.field }}` | Dot-path into the last response body (e.g. `body.access.token`, `body.items.0.id`) |
| `${{ headers.content-type }}` | A header value from the last response |
| `${{ response[N].status }}` | HTTP status code of the Nth response (1-based) |
| `${{ response[N].body }}` | Raw body of the Nth response |
| `${{ response[N].body.some.field }}` | Dot-path into the Nth response body |
| `${{ response[N].headers.name }}` | A header value from the Nth response |

The bare `body`/`status`/`headers.*` forms always refer to the **last** response. Use `response[N].*` when you need to reach an earlier step in the chain.

#### Two-step example

`login.json` — obtain an access token:

```json
{
  "method": "POST",
  "url": "https://dummyjson.com/auth/login",
  "headers": {
    "content-type": "application/json"
  },
  "body": "{\"username\": \"emilys\", \"password\": \"emilyspass\"}"
}
```

`whoami.json` — fetch the current user's profile, injecting the token from the login response:

```json
{
  "method": "GET",
  "url": "https://dummyjson.com/user/me",
  "headers": {
    "Authorization": "Bearer ${{ body.accessToken }}"
  }
}
```

```bash
reel load login.json send then whoami.json
```

#### Three-step example: re-using an earlier response

The third step needs the access token from the login response, not from the profile response that immediately preceded it. Use `response[1].*` to reach back to the first response.

`search_users.json` — search for users, authenticated with the token from step 1:

```json
{
  "method": "GET",
  "url": "https://dummyjson.com/users/search?q=John",
  "headers": {
    "Authorization": "Bearer ${{ response[1].body.accessToken }}"
  }
}
```

```bash
reel load login.json send then whoami.json then search_users.json
```

If a placeholder cannot be resolved (missing key, non-JSON body, out-of-range index) the chain aborts immediately with an error.

## Output

- Response status is written to **stderr** (`200 OK`)
- Response body is written to **stdout**
- `show`, confirmations (`Request loaded from: …`, `Session cleared.`), and all diagnostic messages go to **stderr**
- `response headers` writes headers to **stdout** (it is data, not a status message)

This makes it easy to pipe the body while still seeing the status:

```bash
reel send 2>/dev/null | jq .
reel send | jq .name
```

The response is also saved to the session, so you can inspect it later
with `reel response body` without re-sending the request.

## Session file format

Sessions are stored as plain JSON and can be edited or version-controlled:

```json
{
  "method": "POST",
  "url": "https://api.example.com/users",
  "headers": {
    "Content-Type": "application/json",
    "Authorization": "Bearer mytoken"
  },
  "body": "{\"name\": \"Alice\"}",
  "responses": [
    {
      "status": 201,
      "headers": {
        "content-type": "application/json",
        "content-length": "42"
      },
      "body": "{\"id\": 1, \"name\": \"Alice\"}"
    }
  ]
}
```

The `responses` array is written automatically after each `send` and can be omitted when creating preset files — it will be populated on first use.

> **Note:** Session files and preset files store credentials (e.g. `Authorization` headers) in plaintext. Avoid committing preset files that contain real tokens to version control.

## Dependencies

- [reqwest](https://github.com/seanmonstar/reqwest) — HTTP client
- [serde](https://serde.rs/) / [serde_json](https://github.com/serde-rs/json) — JSON serialization
- [dirs](https://github.com/dirs-dev/dirs-rs) — home directory lookup
