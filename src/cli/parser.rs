use std::path::PathBuf;

use anyhow::{Result, bail};

use super::commands::{Command, GlobalFlags, ResponseTarget, ResponseView};

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

pub fn parse_args(args: &[String]) -> Result<(Vec<Command>, GlobalFlags)> {
    let insecure = args.iter().any(|a| a == "--insecure" || a == "insecure");
    let fail_on_error = args.iter().any(|a| a == "fail");
    let dry_run = args.iter().any(|a| a == "--dry-run" || a == "dry-run");
    let flags = GlobalFlags {
        insecure,
        fail_on_error,
        dry_run,
    };

    let mut commands = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "send" => {
                commands.push(Command::Send);
                i += 1;
            }
            "then" => {
                if i + 1 >= args.len() {
                    bail!("error: 'then' requires a file path");
                }
                commands.push(Command::Then(PathBuf::from(&args[i + 1])));
                i += 2;
            }
            "show" => {
                commands.push(Command::Show);
                i += 1;
            }
            "response" => {
                let rest = &args[i + 1..];
                let (target, target_consumed) = parse_response_target(rest)?;
                let (view, view_consumed) = parse_response_view(&rest[target_consumed..]);
                commands.push(Command::Response(target, view));
                i += 1 + target_consumed + view_consumed;
            }
            "reset" => {
                commands.push(Command::Reset);
                i += 1;
            }
            "fail" | "--insecure" | "insecure" | "--dry-run" | "dry-run" => {
                i += 1;
            }
            "method" => {
                if i + 1 < args.len() {
                    commands.push(Command::Method(args[i + 1].to_uppercase()));
                    i += 2;
                } else {
                    bail!("error: 'method' requires a value");
                }
            }
            "url" => {
                if i + 1 < args.len() {
                    commands.push(Command::Url(args[i + 1].clone()));
                    i += 2;
                } else {
                    bail!("error: 'url' requires a value");
                }
            }
            "header" => {
                if i + 1 >= args.len() {
                    bail!("error: 'header' requires KEY:VALUE or KEY VALUE");
                }
                let next = &args[i + 1];
                if let Some(pos) = next.find(':') {
                    let key = next[..pos].trim().to_string();
                    if key.is_empty() {
                        bail!("error: header key cannot be empty (use KEY:VALUE or KEY VALUE)");
                    }
                    let val = next[pos + 1..].trim().to_string();
                    commands.push(Command::Header(key, val));
                    i += 2;
                } else if i + 2 < args.len() {
                    commands.push(Command::Header(next.to_string(), args[i + 2].clone()));
                    i += 3;
                } else {
                    bail!("error: 'header' requires KEY:VALUE or KEY VALUE");
                }
            }
            "header-rm-all" => {
                commands.push(Command::HeaderRmAll);
                i += 1;
            }
            "header-rm" => {
                if i + 1 < args.len() {
                    commands.push(Command::HeaderRm(args[i + 1].to_lowercase()));
                    i += 2;
                } else {
                    bail!("error: 'header-rm' requires a header name");
                }
            }
            "body" => {
                if i + 1 < args.len() {
                    commands.push(Command::Body(args[i + 1].clone()));
                    i += 2;
                } else {
                    bail!("error: 'body' requires a value");
                }
            }
            "save" => {
                if i + 1 < args.len() {
                    commands.push(Command::Save(PathBuf::from(&args[i + 1])));
                    i += 2;
                } else {
                    bail!("error: 'save' requires a path");
                }
            }
            "load" => {
                if i + 1 < args.len() {
                    commands.push(Command::Load(PathBuf::from(&args[i + 1])));
                    i += 2;
                } else {
                    bail!("error: 'load' requires a path");
                }
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
