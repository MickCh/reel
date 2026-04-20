# reel

A stateful HTTP client for the command line. Like `curl`, but it remembers your settings across invocations — per terminal window.

The name comes from a film reel: each request is a frame, and the session threads them together into a single strip.

## How sessions work

Each terminal window runs its own shell process. `reel` uses the parent shell's PID as a session key, storing state in `~/.reel/sessions/<ppid>.json`. This means:

- Settings persist between invocations **in the same terminal window**
- Different terminal windows have **independent sessions**
- A session disappears naturally when the shell exits

## Installation

```bash
cargo build --release
sudo cp target/release/reel /usr/local/bin/
```

## Commands

Commands can be combined freely in a single invocation.

| Command | Description |
|---|---|
| `method <METHOD>` | Set HTTP method (`GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`) |
| `url <URL>` | Set request URL |
| `header <KEY:VALUE>` | Add a header (`KEY:VALUE` or `KEY VALUE`) |
| `header-rm <KEY>` | Remove a header by name |
| `body <BODY>` | Set request body |
| `send` | Send the request using current session state |
| `then <PATH>` | Load preset file, interpolate `${{ expr }}` from last response, and send |
| `show` | Print the current session state |
| `response [N] [body\|headers]` | Show Nth response (default: last); full JSON, body only, or headers only |
| `responses` | Show all responses from the last chain as a JSON array |
| `reset` | Clear all session state |
| `load <PATH>` | Load state from a JSON file |
| `save <PATH>` | Save current session state to a JSON file |

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

### Managing headers

```bash
reel header "Content-Type: application/json"
reel header Authorization "Bearer mytoken"

# Remove a single header without touching the rest of the session
reel header-rm Authorization

# Both formats for adding are equivalent:
reel header "Authorization: Bearer token"
reel header Authorization "Bearer token"
```

### Inspect responses

The session stores responses automatically. A plain `send` replaces the stored list with a single response; a chain (`send then … then …`) accumulates one response per step.

```bash
reel response           # full JSON of the last response
reel response headers   # status code + response headers
reel response body      # raw response body

# pipe-friendly
reel response body | jq .name
reel response body | jq '.users[] | .email'

# access a specific step in a chain (0-based index)
reel response 0 body    # body of the first response
reel response 1         # full JSON of the second response

# see all responses at once
reel responses
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

### Chaining requests with `then`

`then` loads a preset file, fills in `${{ expr }}` placeholders using the previous response, and sends the request immediately. This lets you chain dependent calls without scripting.

The first request in a chain is loaded explicitly with `load` and sent with `send`. This is intentional — it lets you inspect or modify the session state before committing to the chain.

```bash
reel load login.json send then dashboard.json
```

Supported expressions in preset files:

| Expression | Resolves to |
|---|---|
| `${{ status }}` | HTTP status code of the previous response |
| `${{ body }}` | Raw response body |
| `${{ body.some.field }}` | Dot-path into a JSON body (e.g. `body.access.token`, `body.items.0.id`) |
| `${{ headers.content-type }}` | A response header value |

Example preset (`step2.json`) that uses a token from the previous response:

```json
{
  "method": "GET",
  "url": "https://api.example.com/profile",
  "headers": {
    "Authorization": "Bearer ${{ body.access_token }}"
  }
}
```

If a placeholder cannot be resolved the chain aborts immediately with an error.

## Output

- Response status is written to **stderr** (`200 OK`)
- Response body is written to **stdout**

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
  "last": {
    "status": 201,
    "headers": {
      "content-type": "application/json",
      "content-length": "42"
    },
    "body": "{\"id\": 1, \"name\": \"Alice\"}"
  }
}
```

The `last` field is written automatically after each `send` and can be omitted when creating preset files — it will be populated on first use.

## Dependencies

- [reqwest](https://github.com/seanmonstar/reqwest) — HTTP client
- [serde](https://serde.rs/) / [serde_json](https://github.com/seanmonstar/reqwest) — JSON serialization
- [dirs](https://github.com/dirs-dev/dirs-rs) — home directory lookup
