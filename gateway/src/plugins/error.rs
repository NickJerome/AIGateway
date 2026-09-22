//! Stable management failure categories; error text is diagnostic context only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    NotFound,
    RouteConflict,
    InvalidPackage,
    InvalidConfig,
    UploadLimit,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotFound => "plugin not found",
            Self::RouteConflict => "route already assigned",
            Self::InvalidPackage => "invalid plugin package",
            Self::InvalidConfig => "invalid plugin configuration",
            Self::UploadLimit => "plugin upload exceeds limit",
        })
    }
}
impl std::error::Error for Error {}
