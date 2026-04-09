use std::fmt;

/// Errors from the Pulumi engine.
#[derive(Debug)]
pub enum Error {
    /// gRPC transport error.
    Transport(tonic::transport::Error),
    /// gRPC status error from a provider operation.
    ProviderStatus(tonic::Status),
    /// The user program exited with a non-zero status.
    ProgramFailed {
        code: i32,
        stdout: String,
        stderr: String,
    },
    /// Failed to spawn the user program.
    Spawn(std::io::Error),
    /// Secret encryption or decryption error.
    Secrets(crate::secrets::SecretsError),
    /// A custom error.
    Custom(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Transport(e) => write!(f, "gRPC transport error: {e}"),
            Error::ProviderStatus(s) => write!(f, "provider error: {s}"),
            Error::ProgramFailed { code, stderr, .. } => {
                write!(f, "program failed (exit code {code}):\n{stderr}")
            }
            Error::Spawn(e) => write!(f, "failed to spawn program: {e}"),
            Error::Secrets(e) => write!(f, "secrets error: {e}"),
            Error::Custom(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<tonic::transport::Error> for Error {
    fn from(e: tonic::transport::Error) -> Self {
        Error::Transport(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Spawn(e)
    }
}

impl From<tonic::Status> for Error {
    fn from(s: tonic::Status) -> Self {
        Error::ProviderStatus(s)
    }
}

impl From<crate::secrets::SecretsError> for Error {
    fn from(e: crate::secrets::SecretsError) -> Self {
        Error::Secrets(e)
    }
}

/// A specialized `Result` type for engine operations.
pub type Result<T> = std::result::Result<T, Error>;
