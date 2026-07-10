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
    // Extra attempts for send/then on transient failure (network error or
    // 5xx). 0 = no retries.
    pub retry: u32,
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
