mod cli;
mod http;
mod model;
mod session;
mod template;

use std::env;

use session::SessionStore;

fn main() {
    // Rust ignores SIGPIPE by default, which turns `reel send | head` into a
    // panic on the broken pipe. Restore the default so the process exits
    // quietly like any other CLI tool (the session is saved before any stdout
    // write, so no state is lost).
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        cli::print_usage();
        return;
    }

    let (commands, flags) = match cli::parse_args(&args) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    if flags.help {
        cli::print_usage();
        return;
    }
    if flags.version {
        println!("reel {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    session::cleanup_old_sessions();
    let session = match session::FileSessionStore::new() {
        Ok(session) => session,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };
    let mut state = session.load();

    let http = match http::ReqwestClient::new(flags.insecure) {
        Ok(http) => http,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };
    let presets = session::FilePresetStore;

    match cli::run_commands(commands, &flags, &mut state, &http, &session, &presets) {
        Ok(result) => {
            if result.do_show {
                cli::show_state(&state, &session);
            }
            if let Some((target, view)) = result.do_response
                && !cli::show_response(&state, target, view)
            {
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
}
