/// The error type for Automation API operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The `pulumi` CLI binary was not found on `$PATH`.
    #[error("pulumi CLI not found: {0}")]
    CliNotFound(#[source] std::io::Error),

    /// A `pulumi` CLI command exited with a non-zero status.
    #[error("pulumi {command} failed (exit code {code}):\n{stderr}")]
    CommandFailed {
        command: String,
        code: i32,
        stdout: String,
        stderr: String,
    },

    /// Failed to spawn or communicate with the CLI process.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Failed to parse CLI output as JSON.
    #[error("failed to parse CLI output: {0}")]
    Json(#[from] serde_json::Error),

    /// The requested stack was not found.
    #[error("stack not found: {0}")]
    StackNotFound(String),

    /// The stack already exists.
    #[error("stack already exists: {0}")]
    StackAlreadyExists(String),

    /// A custom error.
    #[error("{0}")]
    Custom(String),
}

/// A specialized `Result` type for Automation API operations.
pub type Result<T> = std::result::Result<T, Error>;
