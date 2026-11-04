use std::path::Path;

/// Errors raised by boilersync-core.
///
/// The variants mirror the exception classes of the original Python
/// implementation so the CLI can keep the same exit codes: usage errors exit
/// with status 2, everything else with status 1.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The command was invoked in a way that is not supported.
    #[error("{0}")]
    Usage(String),
    /// A template, repository, project file or metadata file is missing.
    #[error("{0}")]
    NotFound(String),
    /// A target already exists where an empty location is required.
    #[error("{0}")]
    AlreadyExists(String),
    /// Invalid template metadata, project metadata or template reference.
    #[error("{0}")]
    Invalid(String),
    /// A git command failed or a repository is in an unexpected state.
    #[error("{0}")]
    Git(String),
    /// A template hook or push step could not be completed.
    #[error("{0}")]
    Runtime(String),
    /// Any other failure with a human-readable message.
    #[error("{0}")]
    Other(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Template(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Other(err.to_string())
    }
}

impl From<minijinja::Error> for Error {
    fn from(err: minijinja::Error) -> Self {
        Error::Template(err.to_string())
    }
}

impl Error {
    /// Wrap an I/O error with the path it concerned.
    pub fn io(path: &Path, err: std::io::Error) -> Self {
        Error::Other(format!("{}: {}", path.display(), err))
    }
}
