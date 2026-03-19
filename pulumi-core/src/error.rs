use std::fmt;

/// The error type for Pulumi operations.
#[derive(Debug)]
pub enum Error {
    /// A gRPC transport error.
    Transport(tonic::transport::Error),
    /// A gRPC status error returned by the engine or monitor.
    Rpc(tonic::Status),
    /// A missing required environment variable.
    MissingEnv(&'static str),
    /// A serialization or deserialization error.
    Serde(serde_json::Error),
    /// A resource registration failure.
    ResourceFailed { urn: String },
    /// A provider invoke returned check failures.
    InvokeFailure { token: String, failures: Vec<(String, String)> },
    /// A custom error message.
    Custom(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Transport(e) => write!(f, "gRPC transport error: {e}"),
            Error::Rpc(e) => write!(f, "gRPC error: {e}"),
            Error::MissingEnv(var) => write!(f, "missing environment variable: {var}"),
            Error::Serde(e) => write!(f, "serialization error: {e}"),
            Error::ResourceFailed { urn } => write!(f, "resource registration failed: {urn}"),
            Error::InvokeFailure { token, failures } => {
                write!(f, "invoke {token} failed:")?;
                for (prop, reason) in failures {
                    write!(f, "\n  {prop}: {reason}")?;
                }
                Ok(())
            }
            Error::Custom(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Transport(e) => Some(e),
            Error::Rpc(e) => Some(e),
            Error::Serde(e) => Some(e),
            _ => None,
        }
    }
}

impl From<tonic::transport::Error> for Error {
    fn from(e: tonic::transport::Error) -> Self {
        Error::Transport(e)
    }
}

impl From<tonic::Status> for Error {
    fn from(e: tonic::Status) -> Self {
        Error::Rpc(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Serde(e)
    }
}

/// A specialized `Result` type for Pulumi operations.
pub type Result<T> = std::result::Result<T, Error>;
