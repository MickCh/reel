use std::path::PathBuf;

use anyhow::{Result, bail};

use super::commands::{Command, GlobalFlags, ResponseTarget, ResponseView};

// Cursor over the raw argument tokens. Commands consume their values through
// `value_for`, which yields a uniform "'<cmd>' requires ..." error when the
// value is missing.
struct Tokens<'a> {
    args: &'a [String],
    pos: usize,
}

impl<'a> Tokens<'a> {
    fn new(args: &'a [String]) -> Self {
        Self { args, pos: 0 }
    }

    fn next(&mut self) -> Option<&'a str> {
        let token = self.args.get(self.pos)?;
        self.pos += 1;
        Some(token)
    }

    fn value_for(&mut self, cmd: &str, expected: &str) -> Result<&'a str> {
        self.next()
            .ok_or_else(|| anyhow::anyhow!("error: '{}' requires {}", cmd, expected))
    }

    // Unconsumed tokens — for commands with optional trailing modifiers.
    fn rest(&self) -> &'a [String] {
        &self.args[self.pos..]
    }

    fn advance(&mut self, n: usize) {
        self.pos += n;
    }
}

fn parse_response_view(args: &[String]) -> (ResponseView, usize) {
    match args.first().map(String::as_str) {
        Some("body") => (ResponseView::Body, 1),
        Some("headers") => (ResponseView::Headers, 1),
        _ => (ResponseView::Full, 0),
    }
}

fn parse_response_target(args: &[String]) -> Result<(ResponseTarget, usize)> {
    match args.first().map(String::as_str) {
        Some("all") => Ok((ResponseTarget::All, 1)),
        Some(token) if token != "body" && token != "headers" => {
            if let Ok(n) = token.parse::<usize>() {
                if n == 0 {
                    bail!("error: response indices start at 1");
                }
                Ok((ResponseTarget::Index(n - 1), 1))
            } else {
                Ok((ResponseTarget::Last, 0))
            }
        }
        _ => Ok((ResponseTarget::Last, 0)),
    }
}

// Variable names must stay usable inside a `${{ var.<name> }}` placeholder,
// so the charset excludes anything template syntax gives meaning to (dots,
// pipes, braces, whitespace).
fn validate_var_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!(
            "error: invalid variable name '{}' (allowed: letters, digits, '_', '-')",
            name
        );
    }
    Ok(())
}

fn parse_header(tokens: &mut Tokens) -> Result<Command> {
    let first = tokens.value_for("header", "KEY:VALUE or KEY VALUE")?;
    if let Some((key, value)) = first.split_once(':') {
        let key = key.trim();
        if key.is_empty() {
            bail!("error: header key cannot be empty (use KEY:VALUE or KEY VALUE)");
        }
        Ok(Command::Header(key.to_string(), value.trim().to_string()))
    } else {
        let value = tokens.value_for("header", "KEY:VALUE or KEY VALUE")?;
        Ok(Command::Header(first.to_string(), value.to_string()))
    }
}

pub fn parse_args(args: &[String]) -> Result<(Vec<Command>, GlobalFlags)> {
    // Global flags are set from their own match arms in the loop below, so
    // they only count in command position — a token consumed as a value
    // (`header X-Mode insecure`, `body fail`) must never flip a flag,
    // especially not one that disables TLS verification. They stay
    // position-independent among commands because all flags take effect only
    // after parsing completes.
    let mut flags = GlobalFlags {
        insecure: false,
        fail_on_error: false,
        dry_run: false,
        help: false,
        version: false,
        retry: 0,
        until: None,
        delay_secs: 1,
    };

    let mut commands = Vec::new();
    // Index just past the last verb shortcut's Method+Url — the earliest
    // position the implied send may take.
    let mut verb_end: Option<usize> = None;
    let mut tokens = Tokens::new(args);
    while let Some(token) = tokens.next() {
        match token {
            // Verb shortcuts: `reel get <URL>` = method + url + one implied
            // send, appended after all commands so modifiers written after
            // the verb (body, header, ...) still apply to the request.
            "get" | "post" | "put" | "patch" | "delete" | "head" | "options" => {
                // A second verb would silently overwrite the first request
                // without ever sending it — always a mistake.
                if verb_end.is_some() {
                    anyhow::bail!(
                        "error: '{}' after an earlier verb shortcut — only one per invocation (chain requests with 'then', or use 'method'/'url'/'send')",
                        token
                    );
                }
                let url = tokens.value_for(token, "a URL")?;
                commands.push(Command::Method(token.to_uppercase()));
                commands.push(Command::Url(url.to_string()));
                verb_end = Some(commands.len());
            }
            "fail" => flags.fail_on_error = true,
            "--insecure" | "insecure" => flags.insecure = true,
            "--dry-run" | "dry-run" => flags.dry_run = true,
            "help" | "-h" | "--help" => flags.help = true,
            "-V" | "--version" => flags.version = true,
            "--retry" => {
                let value = tokens.value_for("--retry", "a number of retries")?;
                flags.retry = value
                    .parse()
                    .map_err(|_| anyhow::anyhow!("error: invalid --retry count '{}'", value))?;
            }
            "--until" => {
                let condition =
                    tokens.value_for("--until", "a condition (e.g. --until 'status == 200')")?;
                flags.until = Some(condition.to_string());
            }
            "--delay" => {
                let value = tokens.value_for("--delay", "a number of seconds")?;
                flags.delay_secs = value
                    .parse()
                    .map_err(|_| anyhow::anyhow!("error: invalid --delay seconds '{}'", value))?;
            }
            "send" => commands.push(Command::Send),
            "show" => commands.push(Command::Show),
            "curl" => commands.push(Command::Curl),
            "reset" => commands.push(Command::Reset),
            "header-rm-all" => commands.push(Command::HeaderRmAll),
            "method" => {
                let value = tokens.value_for("method", "a value")?;
                commands.push(Command::Method(value.to_uppercase()));
            }
            "url" => {
                let value = tokens.value_for("url", "a value")?;
                commands.push(Command::Url(value.to_string()));
            }
            "body" => {
                let value = tokens.value_for("body", "a value")?;
                commands.push(Command::Body(value.to_string()));
            }
            "header" => commands.push(parse_header(&mut tokens)?),
            "var" => {
                let name = tokens.value_for("var", "a name and a value")?;
                validate_var_name(name)?;
                let value = tokens.value_for("var", "a value")?;
                commands.push(Command::Var(name.to_string(), value.to_string()));
            }
            "var-rm" => {
                let name = tokens.value_for("var-rm", "a variable name")?;
                commands.push(Command::VarRm(name.to_string()));
            }
            "header-rm" => {
                let name = tokens.value_for("header-rm", "a header name")?;
                commands.push(Command::HeaderRm(name.to_lowercase()));
            }
            "then" => {
                let path = tokens.value_for("then", "a file path")?;
                commands.push(Command::Then(PathBuf::from(path)));
            }
            "expect" => {
                let condition =
                    tokens.value_for("expect", "a condition (e.g. expect 'status == 200')")?;
                commands.push(Command::Expect(condition.to_string()));
            }
            "save" => {
                let path = tokens.value_for("save", "a path")?;
                commands.push(Command::Save(PathBuf::from(path)));
            }
            "load" => {
                let path = tokens.value_for("load", "a path")?;
                commands.push(Command::Load(PathBuf::from(path)));
            }
            "response" => {
                let (target, consumed) = parse_response_target(tokens.rest())?;
                tokens.advance(consumed);
                let (view, consumed) = parse_response_view(tokens.rest());
                tokens.advance(consumed);
                commands.push(Command::Response(target, view));
            }
            unknown => {
                bail!(
                    "error: unknown command '{}'\nRun 'reel' with no arguments to see usage.",
                    unknown
                );
            }
        }
    }

    // A verb shortcut implies exactly one send, unless the user already wrote
    // an explicit send/then. It goes before the first expect written after
    // the verb, so assertions check the verb's response; an expect written
    // before the verb keeps asserting on the prior session history.
    if let Some(verb_end) = verb_end
        && !commands
            .iter()
            .any(|c| matches!(c, Command::Send | Command::Then(_)))
    {
        let pos = commands[verb_end..]
            .iter()
            .position(|c| matches!(c, Command::Expect(_)))
            .map_or(commands.len(), |i| verb_end + i);
        commands.insert(pos, Command::Send);
    }

    Ok((commands, flags))
}
