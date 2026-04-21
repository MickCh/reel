mod cli;
mod http;
mod model;
mod session;
mod template;

use std::env;

use session::SessionStore;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") || args[0] == "help" {
        cli::print_usage();
        return;
    }

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("reel {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    session::cleanup_old_sessions();
    let session = session::FileSessionStore::new();
    let mut state = session.load();

    let (commands, flags) = match cli::parse_args(&args) {
        Ok(result) => result,
        Err(()) => std::process::exit(1),
    };

    let http = http::ReqwestClient::new(flags.insecure);

    match cli::run_commands(commands, &flags, &mut state, &http, &session) {
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
        Err(()) => std::process::exit(1),
    }
}
