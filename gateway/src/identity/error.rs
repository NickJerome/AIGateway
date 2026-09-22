//! Domain failures; the HTTP adapter owns status-code mapping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    InvalidInput,
    Unauthenticated,
    Forbidden,
    NotFound,
    Conflict,
    Busy,
}
#[derive(Debug)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: &'static str,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message)
    }
}
impl std::error::Error for Error {}
pub(crate) fn fail(kind: ErrorKind, message: &'static str) -> anyhow::Error {
    Error { kind, message }.into()
}
pub(crate) fn invalid() -> anyhow::Error {
    fail(ErrorKind::Unauthenticated, "invalid or expired credentials")
}
