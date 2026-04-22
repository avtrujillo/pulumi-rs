use std::fmt;

/// The error type for Automation API operations.
#[derive(Debug)]
pub enum Error {
    /// The `pulumi` CLI binary was not found on `$PATH`.
    CliNotFound(std::io::Error),

    /// A `pulumi` CLI command exited with a non-zero status.
    CommandFailed {
        command: String,
        code: i32,
        stdout: String,
        stderr: String,
    },

    /// Failed to spawn or communicate with the CLI process.
    Io(std::io::Error),

    /// Failed to parse CLI output as JSON.
    Json(serde_json::Error),

    /// The requested stack was not found.
    StackNotFound(String),

    /// The stack already exists.
    StackAlreadyExists(String),

    /// A custom error.
    Custom(String),
}

/// Extracts the most actionable lines from CLI stderr output.
///
/// If stderr contains any lines starting with `error:` (case-insensitive),
/// returns only those lines. Otherwise returns the full stderr to avoid
/// discarding context when the CLI doesn't use the standard prefix.
fn extract_error_lines(stderr: &str) -> String {
    let error_lines: Vec<&str> = stderr
        .lines()
        .filter(|l| l.trim_start().to_lowercase().starts_with("error:"))
        .collect();
    if error_lines.is_empty() {
        stderr.to_string()
    } else {
        error_lines.join("\n")
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::CliNotFound(e) => write!(f, "pulumi CLI not found: {e}"),
            Error::CommandFailed { command, code, stderr, .. } => {
                write!(
                    f,
                    "pulumi {command} failed (exit code {code}):\n{}",
                    extract_error_lines(stderr)
                )
            }
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Json(e) => write!(f, "failed to parse CLI output: {e}"),
            Error::StackNotFound(name) => write!(f, "stack not found: {name}"),
            Error::StackAlreadyExists(name) => write!(f, "stack already exists: {name}"),
            Error::Custom(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::CliNotFound(e) | Error::Io(e) => Some(e),
            Error::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}

#[cfg(feature = "native-engine")]
impl From<pulumi_engine::Error> for Error {
    fn from(e: pulumi_engine::Error) -> Self {
        match e {
            pulumi_engine::Error::ProgramFailed { code, stdout, stderr } => {
                Error::CommandFailed {
                    command: "<native engine>".to_string(),
                    code,
                    stdout,
                    stderr,
                }
            }
            other => Error::Custom(other.to_string()),
        }
    }
}

/// A specialized `Result` type for Automation API operations.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_error_lines_returns_only_error_lines() {
        let stderr = "Updating (dev):\n\n    aws:s3:Bucket  my-bucket  creating\n\
                     error: update failed\nerror: provider error: NoSuchBucket\n\n\
                     Resources:\n    1 error";
        let extracted = extract_error_lines(stderr);
        assert_eq!(extracted, "error: update failed\nerror: provider error: NoSuchBucket");
    }

    #[test]
    fn extract_error_lines_full_stderr_when_no_prefix() {
        let stderr = "Something went wrong\nbut no error: prefix here";
        let extracted = extract_error_lines(stderr);
        assert_eq!(extracted, stderr);
    }

    #[test]
    fn extract_error_lines_empty() {
        assert_eq!(extract_error_lines(""), "");
    }

    #[test]
    fn command_failed_display_extracts_errors() {
        let err = Error::CommandFailed {
            command: "up".to_string(),
            code: 1,
            stdout: String::new(),
            stderr: "progress output\nerror: something failed\nmore output".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("pulumi up failed (exit code 1)"));
        assert!(msg.contains("error: something failed"));
        assert!(!msg.contains("progress output"));
        assert!(!msg.contains("more output"));
    }

    #[test]
    fn command_failed_display_full_stderr_when_no_error_prefix() {
        let err = Error::CommandFailed {
            command: "preview".to_string(),
            code: 255,
            stdout: String::new(),
            stderr: "connection refused".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("connection refused"));
    }

    #[test]
    fn cli_not_found_display() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "No such file");
        let err = Error::CliNotFound(io_err);
        assert!(err.to_string().starts_with("pulumi CLI not found:"));
    }

    #[test]
    fn stack_not_found_display() {
        let err = Error::StackNotFound("my-stack".to_string());
        assert_eq!(err.to_string(), "stack not found: my-stack");
    }

    #[test]
    fn stack_already_exists_display() {
        let err = Error::StackAlreadyExists("dev".to_string());
        assert_eq!(err.to_string(), "stack already exists: dev");
    }

    #[test]
    fn custom_display() {
        let err = Error::Custom("something custom".to_string());
        assert_eq!(err.to_string(), "something custom");
    }

    #[test]
    fn from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::Io(_)));
    }

    #[test]
    fn from_json_error() {
        let json_err = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        let err: Error = json_err.into();
        assert!(matches!(err, Error::Json(_)));
    }

    #[test]
    fn error_source_chain_cli_not_found() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let err = Error::CliNotFound(io_err);
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn error_source_chain_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe");
        let err = Error::Io(io_err);
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn error_source_chain_stack_not_found_is_none() {
        let err = Error::StackNotFound("x".into());
        assert!(std::error::Error::source(&err).is_none());
    }

    #[test]
    fn extract_case_insensitive() {
        let stderr = "some output\nError: capital E works too\nmore";
        let extracted = extract_error_lines(stderr);
        assert_eq!(extracted, "Error: capital E works too");
    }
}
