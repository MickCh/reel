use std::path::PathBuf;

#[derive(Debug)]
pub enum ResponseTarget {
    Last,
    Index(usize),
    All,
}

#[derive(Debug)]
pub enum ResponseView {
    Full,
    Body,
    Headers,
}

#[derive(Debug)]
pub enum Command {
    Method(String),
    Url(String),
    Header(String, String),
    HeaderRm(String),
    HeaderRmAll,
    Body(String),
    BodyRm,
    CookieRm(String),
    Timeout(u64),
    Var(String, String),
    VarRm(String),
    Send,
    Then(PathBuf),
    Expect(String),
    Curl,
    Show,
    Response(ResponseTarget, ResponseView),
    Reset,
    Save(PathBuf),
    Load(PathBuf),
}

#[derive(Debug)]
pub struct GlobalFlags {
    pub insecure: bool,
    pub fail_on_error: bool,
    pub dry_run: bool,
    // -h/--help/help and -V/--version anywhere in command position; main
    // prints usage/version and exits without running any command.
    pub help: bool,
    pub version: bool,
    // Follow 3xx redirects (opt-in, like curl -L). The redirect loop lives in
    // cli/runner.rs so the cookie jar sees every hop.
    pub follow: bool,
    // Extra attempts for send/then on transient failure (network error or
    // 5xx). None = flag absent; an explicit `--retry 0` stays distinguishable
    // from the default so it caps an `--until` poll at a single attempt.
    pub retry: Option<u32>,
    // Poll: repeat send/then until this condition passes (attempt budget:
    // `retry + 1` when --retry is given, otherwise 10).
    pub until: Option<String>,
    // Seconds to sleep between attempts.
    pub delay_secs: u64,
}

#[derive(Debug)]
pub struct ParseResult {
    pub do_show: bool,
    pub do_response: Option<(ResponseTarget, ResponseView)>,
}
