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

Session files record the shell's start time alongside its PID, so a session is never accidentally inherited by an unrelated process that later receives the same PID. Sessions whose shell has exited are cleaned up automatically on the next `reel` invocation; a session belonging to a running shell is never removed, no matter how old (long-lived `tmux`/`screen` shells are safe).

### Named sessions (`REEL_SESSION`)

Setting the `REEL_SESSION` environment variable overrides the PID-based key with a name of your choice (1–64 characters from `A-Za-z0-9._-`):

```bash
export REEL_SESSION=deploy   # every reel call in this shell now shares session "deploy"
REEL_SESSION=ci reel send    # one-off: use session "ci" for a single invocation
```

This is useful in scripts, CI, or anywhere the parent-PID heuristic doesn't fit — a script normally gets its own session because its interpreter is the parent process, but with `REEL_SESSION` it can share one with the invoking shell or with other scripts. Named sessions are stored as `~/.reel/sessions/named-<name>.json` and are removed automatically after 7 days without use.

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
| `expect <CONDITION>` | Assert on the last response; abort with exit code 1 on failure |
| `--dry-run` | Print the request that would be sent, without sending it (position-independent) |
| `then <PATH>` | Load preset file, interpolate `${{ expr }}` from request/response history and environment, and send |
| `show` | Print the current session state |
| `response [N\|all] [body\|headers]` | Show Nth response (default: last), or all; full JSON, body only, or headers only |
| `reset` | Clear all session state |
| `load <PATH>` | Load state from a JSON file |
| `save <PATH>` | Save current session state to a JSON file |
| `fail` | Exit with code 1 if any `send` or `then` receives a 4xx/5xx response (position-independent) |
| `--insecure` | Skip TLS certificate verification |
| `--retry <N>` | Retry `send`/`then` up to N extra times on network error or 5xx |
| `--until <CONDITION>` | Poll: repeat `send`/`then` until the condition passes |
| `--delay <SECONDS>` | Sleep between retry/poll attempts (default: 1) |

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

`then` loads a preset file, fills in `${{ expr }}` placeholders using the full request/response history and environment variables, and sends the request immediately. This lets you chain dependent calls without scripting.

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
| `${{ request.method }}` / `${{ request.url }}` / `${{ request.body }}` | A field of the **last** request, exactly as it was sent |
| `${{ request.body.some.field }}` | Dot-path into the last request body |
| `${{ request.headers.name }}` | A header value from the last request (case-insensitive) |
| `${{ request[N].url }}` | Any `request` field for the Nth request (1-based) |
| `${{ env.NAME }}` | Value of the environment variable `NAME` |
| `${{ uuid() }}` | A random v4 UUID (fresh per placeholder — handy for idempotency keys) |
| `${{ now() }}` / `${{ now(+3600) }}` | Current Unix timestamp in seconds, optionally shifted |
| `${{ base64('user:pass') }}` / `${{ base64(env.CREDS) }}` | Base64 of a quoted literal or of a nested expression (e.g. for Basic auth) |
| `${{ env.API_HOST \| default: localhost:8080 }}` | Fallback value used when the expression cannot be resolved |

The bare `body`/`status`/`headers.*` forms always refer to the **last** response, and bare `request.*` to the **last** request. Use `response[N].*` / `request[N].*` when you need to reach an earlier step in the chain — requests and responses share the same index, so `request[N]` is the request that produced `response[N]`.

Request values are captured **after** interpolation, so `${{ request.* }}` reflects what was actually sent (useful for echoing back an id, correlation header, or URL you built in a previous step).

`${{ env.NAME }}` reads a variable from the process environment — handy for keeping secrets out of preset files (`"Authorization": "Bearer ${{ env.API_TOKEN }}"`). Because it does not depend on history, a preset that uses only `env.*` interpolates fine even as the first request in a chain. A referenced variable that is not set aborts the chain — unless the placeholder carries a `| default: <value>` fallback, which is used whenever the expression cannot be resolved:

```json
{ "url": "https://${{ env.API_HOST | default: localhost:8080 }}/users" }
```

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

#### Environment variables and request references

Preset files can pull secrets from the environment and echo back values from earlier requests:

`create.json` — create a resource, authenticating with a token from the environment:

```json
{
  "method": "POST",
  "url": "https://api.example.com/orders",
  "headers": {
    "content-type": "application/json",
    "Authorization": "Bearer ${{ env.API_TOKEN }}",
    "Idempotency-Key": "order-42"
  },
  "body": "{\"item\": \"widget\"}"
}
```

`confirm.json` — confirm using the id from the response and the idempotency key from the previous request:

```json
{
  "method": "POST",
  "url": "https://api.example.com/orders/${{ body.id }}/confirm",
  "headers": {
    "Authorization": "Bearer ${{ env.API_TOKEN }}",
    "Idempotency-Key": "${{ request.headers.Idempotency-Key }}"
  }
}
```

```bash
API_TOKEN=sk-live-... reel load create.json send then confirm.json
```

If a placeholder cannot be resolved (missing key, non-JSON body, out-of-range index, or unset environment variable) the chain aborts immediately with an error.

### Assertions with `expect`

`expect` turns reel into a lightweight API test runner: it evaluates a condition against the response history and aborts the command chain (exit code 1) when it fails. The condition uses the same expressions as templates, without the `${{ }}` wrapper:

```bash
reel send expect 'status == 200'
reel send expect 'body.token'                     # passes if the field exists
reel send expect 'headers.content-type contains json'
reel load login.json send expect 'status == 200' then create.json expect 'status == 201'
```

Supported forms: `<expr>` (must resolve), `<expr> == <value>`, `<expr> != <value>`, `<expr> contains <value>`. Any template expression works, including `response[N].*`, `request.*`, and `env.*`. A failing `expect` stops later commands from running — useful in CI scripts and as a guard before a destructive `then` step.

### Retrying and polling

`--retry N` retries a failed `send`/`then` up to N extra times — a failure being a network error or a 5xx response. 4xx responses are not retried (they are deterministic client errors). Only the final attempt is recorded in the session history.

```bash
reel --retry 3 send                    # tolerate a flaky endpoint
reel --retry 3 --delay 5 send          # wait 5s between attempts
```

`--until CONDITION` turns `send`/`then` into a poll loop: the request repeats until the condition (same syntax as `expect`) passes. The attempt budget is `--retry + 1` when `--retry` is given, otherwise 10. If the budget runs out, the last response is still recorded for inspection, but reel exits with code 1.

```bash
reel url https://api.example.com/jobs/42 --until 'body.state == done' --delay 2 send
reel --until 'status == 200' --retry 30 --delay 10 send    # wait for a deploy
```

Both flags are position-independent and apply to every `send`/`then` in the invocation.

## Cookies

`reel` keeps a cookie jar in the session. Cookies from `Set-Cookie` response headers are stored automatically and sent back on subsequent requests that match the cookie's domain, path, and `Secure` attribute — so a login endpoint that sets a session cookie "just works":

```bash
reel method POST url https://example.com/login body '{"user":"alice"}' send
reel url https://example.com/profile send    # session cookie sent automatically
```

Details:

- A `Cookie` header you set manually (`reel header Cookie ...`) always wins over the jar.
- The jar survives `send` (which only clears the response history) and is cleared by `reset` or when the server expires a cookie (`Max-Age=0`).
- Cookie lifetimes (`Expires`/`Max-Age`) are not tracked otherwise — a cookie lives as long as the session.
- Cookies are visible in `reel show` and stored in the session file (plaintext — same caveat as headers).

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

The `responses` array is written automatically after each `send` and can be omitted when creating preset files — it will be populated on first use. PID-keyed session files also carry an `owner_start_time` field identifying the owning shell; it is ignored in preset files.

> **Note:** Session files and preset files store credentials (e.g. `Authorization` headers) in plaintext (session files are created with `0600` permissions on Unix). Avoid committing preset files that contain real tokens to version control.

## Dependencies

- [reqwest](https://github.com/seanmonstar/reqwest) — HTTP client
- [serde](https://serde.rs/) / [serde_json](https://github.com/serde-rs/json) — JSON serialization
- [dirs](https://github.com/dirs-dev/dirs-rs) — home directory lookup
