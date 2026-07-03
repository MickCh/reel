# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

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

[Unreleased]: https://github.com/MickCh/reel/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/MickCh/reel/releases/tag/v0.1.0
