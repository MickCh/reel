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

#[derive(Debug)]
pub struct ParseResult {
    pub do_show: bool,
    pub do_response: Option<(ResponseTarget, ResponseView)>,
}
