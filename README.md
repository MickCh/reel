# req

A stateful HTTP client for the command line. Like `curl`, but it remembers your settings across invocations — per terminal window.

## How sessions work

Each terminal window runs its own shell process. `req` uses the parent shell's PID as a session key, storing state in `~/.req/sessions/<ppid>.json`. This means:

- Settings persist between invocations **in the same terminal window**
- Different terminal windows have **independent sessions**
- A session disappears naturally when the shell exits

## Installation

```bash
cargo build --release
sudo cp target/release/req /usr/local/bin/
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
| `exec` | Send the request using current session state |
| `show` | Print the current session state |
| `last [body\|headers]` | Show last response: full JSON (default), body only, or headers only |
| `reset` | Clear all session state |
| `file <PATH>` | Load state from a JSON file |
| `save <PATH>` | Save current session state to a JSON file |

## Usage examples

### Build a request step by step

```bash
req url https://api.example.com/users
req method POST
req header "Content-Type: application/json"
req header "Authorization: Bearer mytoken"
req body '{"name": "Alice"}'
req show      # inspect before sending
req exec
```

### One-liner

```bash
req method GET url https://httpbin.org/get exec
```

### Reuse settings across calls

```bash
# Set base config once
req url https://api.example.com/users
req header "Authorization: Bearer mytoken"

# Subsequent calls reuse the stored state
req exec
req url https://api.example.com/posts exec
```

### Managing headers

```bash
req header "Content-Type: application/json"
req header Authorization "Bearer mytoken"

# Remove a single header without touching the rest of the session
req header-rm Authorization

# Both formats for adding are equivalent:
req header "Authorization: Bearer token"
req header Authorization "Bearer token"
```

### Inspect the last response

The session stores the last response automatically after every `exec`.

```bash
req last           # full JSON: { status, headers, body }
req last headers   # status code + response headers
req last body      # raw response body

# pipe-friendly
req last body | jq .name
req last body | jq '.users[] | .email'
```

`last` reads from the stored session state, so it works even after the
original `exec` has finished — no need to re-run the request.

### Save and restore sessions

```bash
# Save current session to a file
req save staging.json

# Restore it later (in any terminal window)
req file staging.json exec

# Set up, save, and execute in one go
req method POST url https://api.example.com save prod.json exec

# Derive a new preset from an existing one
req file prod.json url https://staging.example.com save staging.json
```

This is useful for sharing named presets between terminals or keeping configurations for different environments.

## Output

- Response status is written to **stderr** (`200 OK`)
- Response body is written to **stdout**

This makes it easy to pipe the body while still seeing the status:

```bash
req exec 2>/dev/null | jq .
req exec | jq .name
```

The response is also saved to the session, so you can inspect it later
with `req last body` without re-sending the request.

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

The `last` field is written automatically after each `exec` and can be omitted when creating preset files — it will be populated on first use.

## Dependencies

- [reqwest](https://github.com/seanmonstar/reqwest) — HTTP client
- [serde](https://serde.rs/) / [serde_json](https://github.com/seanmonstar/reqwest) — JSON serialization
- [dirs](https://github.com/dirs-dev/dirs-rs) — home directory lookup
