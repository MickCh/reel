mod cli;
mod http;
mod model;
mod session;
mod template;

use std::env;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        cli::print_usage();
        return;
    }

    let mut state = session::load_state();
    match cli::parse_and_run(&args, &mut state) {
        Ok(result) => {
            if result.do_show {
                cli::show_state(&state);
            }
            if let Some((target, view)) = result.do_response {
                cli::show_response(&state, target, view);
            }
        }
        Err(()) => std::process::exit(1),
    }
}
