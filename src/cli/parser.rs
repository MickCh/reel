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
    // Global flags apply regardless of position; pre-scan the value-less ones
    // here and skip their tokens in the command loop below. Flags that take a
    // value (--retry, --until, --delay) are consumed inside the loop — they
    // are still position-independent because all flags take effect only after
    // parsing completes.
    let mut flags = GlobalFlags {
        insecure: args.iter().any(|a| a == "--insecure" || a == "insecure"),
        fail_on_error: args.iter().any(|a| a == "fail"),
        dry_run: args.iter().any(|a| a == "--dry-run" || a == "dry-run"),
        retry: 0,
        until: None,
        delay_secs: 1,
    };

    let mut commands = Vec::new();
    let mut tokens = Tokens::new(args);
    while let Some(token) = tokens.next() {
        match token {
            "fail" | "--insecure" | "insecure" | "--dry-run" | "dry-run" => {}
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
    Ok((commands, flags))
}
