# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Cookie jar: cookies from `Set-Cookie` response headers are stored in the session and sent automatically on matching requests (domain, path, and `Secure` scoping; `Max-Age=0` deletes). A manually set `Cookie` header always wins. Cookies are shown by `show` and cleared by `reset`.
- `expect <CONDITION>` command: assert on the response history (`status == 200`, `body.token`, `headers.content-type contains json`, …) using template expressions; a failing assertion aborts the chain with exit code 1.
- `--retry <N>` flag: retry `send`/`then` on network errors and 5xx responses; `--until <CONDITION>` flag: poll until a condition passes; `--delay <SECONDS>` flag: pause between attempts (default 1 s). Only the final attempt is recorded in history.
- Template functions: `${{ uuid() }}` (random v4 UUID), `${{ now() }}` / `${{ now(±N) }}` (Unix timestamp with optional offset), and `${{ base64(arg) }}` (base64 of a quoted literal or a nested expression, e.g. `base64(env.CREDS)` for Basic auth).
- Template default values: `${{ expr | default: value }}` uses the fallback when the expression cannot be resolved (unset environment variable, missing history or JSON key) instead of aborting the chain.
- `body @<PATH>` reads the request body from a file and `body -` from stdin (both verbatim); `@@` escapes a literal body starting with `@`.
- Verb shortcuts: `reel get <URL>` (and `post`/`put`/`patch`/`delete`/`head`/`options`) sets the method and URL and sends in one word. The implied send runs after all other commands (so trailing `body`/`header` still apply) and before any `expect`; it is skipped when an explicit `send`/`then` is present.

## [0.1.1] - 2026-07-03

### Added

- Template expressions for request history: `request.method`, `request.url`, `request.body`, `request.body.<dot.path>`, `request.headers.<name>`, and indexed `request[N].*` variants. Requests are captured after interpolation, so they reflect what was actually sent, and share the same 1-based index as responses.
- Template expression `env.<NAME>` for reading environment variables in preset files — keeps secrets out of files and works even as the first request in a chain.

### Changed

- `then` now always attempts interpolation; a placeholder referencing history that does not exist yet (or an unset environment variable) aborts the chain with a specific error instead of a generic "no previous response" pre-check.

## [0.1.0] - 2026-04-22

### Added

- Stateful CLI HTTP client with per-terminal session isolation via parent shell PID
- Commands: `method`, `url`, `header`, `header-rm`, `header-rm-all`, `body`, `send`, `show`, `reset`, `load`, `save`, `response`, `then`
- `--dry-run` flag — prints the request to stderr without sending
- `--insecure` flag — skips TLS certificate verification
- `fail` flag — exits with code 1 on 4xx/5xx responses
- `--help` and `--version` flags
- Request chaining with `then` and `${{ expr }}` template interpolation in preset files
- Template expressions: `status`, `body`, `body.<dot.path>`, `headers.<name>`, and indexed `response[N].*` variants for accessing any step in the chain
- Response history stored in session; `response [N|all] [body|headers]` for inspection
- Automatic session cleanup — sessions older than 7 days are removed at startup
- Cross-platform PPID-based session isolation: Linux/macOS/BSD via `libc::getppid()`, Windows via `CreateToolhelp32Snapshot`
- CI on Linux, macOS, and Windows via GitHub Actions

[Unreleased]: https://github.com/MickCh/reel/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/MickCh/reel/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/MickCh/reel/releases/tag/v0.1.0
