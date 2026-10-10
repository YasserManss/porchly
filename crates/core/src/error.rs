/// Every fallible Porchly operation returns this error. Messages are written for the
/// person using the app: they say what happened and, where possible, what to do next.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    /// The input was not acceptable (bad name, wrong value type, missing parameter…).
    #[error("{0}")]
    Invalid(String),
    /// The thing asked for does not exist.
    #[error("{0}")]
    NotFound(String),
    /// The thing being created already exists.
    #[error("{0}")]
    Conflict(String),
    /// No valid key was presented.
    #[error("A valid key is needed. Ask the owner of this Porchly for one.")]
    Unauthorized,
    /// The key is valid but not allowed to do this.
    #[error("{0}")]
    Forbidden(String),
    /// The input is bigger than Porchly accepts.
    #[error("{0}")]
    TooLarge(String),
    /// The project file was written by a newer Porchly.
    #[error(
        "This project was saved by a newer version of Porchly (format {found}; this version reads up to {supported}). Please update Porchly to open it."
    )]
    NewerFormat { found: u32, supported: u32 },
    /// The database reported a problem. Details are kept out of API responses.
    #[error("Storage problem: {0}")]
    Storage(String),
    /// A bug in Porchly. Details are kept out of API responses.
    #[error("Something went wrong inside Porchly: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// A short machine-readable code, stable across versions (used by the HTTP API).
    pub fn code(&self) -> &'static str {
        match self {
            Error::Invalid(_) => "invalid",
            Error::NotFound(_) => "not_found",
            Error::Conflict(_) => "conflict",
            Error::Unauthorized => "unauthorized",
            Error::Forbidden(_) => "forbidden",
            Error::TooLarge(_) => "too_large",
            Error::NewerFormat { .. } => "newer_format",
            Error::Storage(_) => "storage",
            Error::Internal(_) => "internal",
        }
    }

    /// Whether the message may be shown to an API caller. Storage and internal errors
    /// may contain paths or SQL and are replaced by a generic message.
    pub fn is_public(&self) -> bool {
        !matches!(self, Error::Storage(_) | Error::Internal(_))
    }

    pub fn invalid(msg: impl Into<String>) -> Self {
        Error::Invalid(msg.into())
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Error::NotFound(msg.into())
    }
}
