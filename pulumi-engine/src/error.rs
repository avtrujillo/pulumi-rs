use std::fmt;

/// Errors from the Pulumi engine.
#[derive(Debug)]
pub enum Error {
    /// gRPC transport error.
    Transport(tonic::transport::Error),
    /// The user program exited with a non-zero status.
    ProgramFailed {
        code: i32,
        stdout: String,
        stderr: String,
    },
    /// Failed to spawn the user program.
    Spawn(std::io::Error),
    /// A custom error.
    Custom(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Transport(e) => write!(f, "gRPC transport error: {e}"),
            Error::ProgramFailed { code, stderr, .. } => {
                write!(f, "program failed (exit code {code}):\n{stderr}")
            }
            Error::Spawn(e) => write!(f, "failed to spawn program: {e}"),
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

/// A specialized `Result` type for engine operations.
pub type Result<T> = std::result::Result<T, Error>;
