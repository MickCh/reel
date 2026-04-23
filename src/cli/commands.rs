use std::path::PathBuf;

pub enum ResponseTarget {
    Last,
    Index(usize),
    All,
}

pub enum ResponseView {
    Full,
    Body,
    Headers,
}

pub enum Command {
    Method(String),
    Url(String),
    Header(String, String),
    HeaderRm(String),
    HeaderRmAll,
    Body(String),
    Send,
    Then(PathBuf),
    Show,
    Response(ResponseTarget, ResponseView),
    Reset,
    Save(PathBuf),
    Load(PathBuf),
}

pub struct GlobalFlags {
    pub insecure: bool,
    pub fail_on_error: bool,
    pub dry_run: bool,
}

pub struct ParseResult {
    pub do_show: bool,
    pub do_response: Option<(ResponseTarget, ResponseView)>,
}
