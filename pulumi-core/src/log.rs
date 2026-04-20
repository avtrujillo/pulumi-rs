use crate::connection::{EngineConnection, MonitorConnection};
use crate::context::Context;
use crate::error::Result;
use crate::proto::pulumirpc;

/// Log severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Debug,
    Info,
    Warning,
    Error,
}

impl From<Severity> for i32 {
    fn from(s: Severity) -> i32 {
        match s {
            Severity::Debug => pulumirpc::LogSeverity::Debug as i32,
            Severity::Info => pulumirpc::LogSeverity::Info as i32,
            Severity::Warning => pulumirpc::LogSeverity::Warning as i32,
            Severity::Error => pulumirpc::LogSeverity::Error as i32,
        }
    }
}

/// Sends a log message to the Pulumi engine.
async fn log_message<M: MonitorConnection, E: EngineConnection>(
    ctx: &Context<M, E>,
    severity: Severity,
    message: &str,
    urn: Option<&str>,
    ephemeral: bool,
) -> Result<()> {
    let req = pulumirpc::LogRequest {
        severity: i32::from(severity),
        message: message.to_string(),
        urn: urn.unwrap_or("").to_string(),
        stream_id: 0,
        ephemeral,
    };

    ctx.engine().log(req).await
}

/// Logs a debug message.
pub async fn debug<M: MonitorConnection, E: EngineConnection>(
    ctx: &Context<M, E>,
    message: &str,
    urn: Option<&str>,
) -> Result<()> {
    log_message(ctx, Severity::Debug, message, urn, false).await
}

/// Logs an informational message.
pub async fn info<M: MonitorConnection, E: EngineConnection>(
    ctx: &Context<M, E>,
    message: &str,
    urn: Option<&str>,
) -> Result<()> {
    log_message(ctx, Severity::Info, message, urn, false).await
}

/// Logs a warning message.
pub async fn warn<M: MonitorConnection, E: EngineConnection>(
    ctx: &Context<M, E>,
    message: &str,
    urn: Option<&str>,
) -> Result<()> {
    log_message(ctx, Severity::Warning, message, urn, false).await
}

/// Logs an error message.
pub async fn error<M: MonitorConnection, E: EngineConnection>(
    ctx: &Context<M, E>,
    message: &str,
    urn: Option<&str>,
) -> Result<()> {
    log_message(ctx, Severity::Error, message, urn, false).await
}

/// Logs an ephemeral status message (shown during operations but not persisted).
pub async fn status<M: MonitorConnection, E: EngineConnection>(
    ctx: &Context<M, E>,
    message: &str,
    urn: Option<&str>,
) -> Result<()> {
    log_message(ctx, Severity::Info, message, urn, true).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::pulumirpc;
    use crate::test_support::TestContextBuilder;

    #[test]
    fn test_severity_debug_value() {
        assert_eq!(i32::from(Severity::Debug), pulumirpc::LogSeverity::Debug as i32);
    }

    #[test]
    fn test_severity_info_value() {
        assert_eq!(i32::from(Severity::Info), pulumirpc::LogSeverity::Info as i32);
    }

    #[test]
    fn test_severity_warning_value() {
        assert_eq!(i32::from(Severity::Warning), pulumirpc::LogSeverity::Warning as i32);
    }

    #[test]
    fn test_severity_error_value() {
        assert_eq!(i32::from(Severity::Error), pulumirpc::LogSeverity::Error as i32);
    }

    #[tokio::test]
    async fn test_debug_ok() {
        let tc = TestContextBuilder::new().build();
        debug(tc.context(), "debug message", None).await.unwrap();
    }

    #[tokio::test]
    async fn test_info_ok() {
        let tc = TestContextBuilder::new().build();
        info(tc.context(), "info message", Some("urn:test")).await.unwrap();
    }

    #[tokio::test]
    async fn test_warn_ok() {
        let tc = TestContextBuilder::new().build();
        warn(tc.context(), "warning message", None).await.unwrap();
    }

    #[tokio::test]
    async fn test_error_ok() {
        let tc = TestContextBuilder::new().build();
        error(tc.context(), "error message", Some("urn:res")).await.unwrap();
    }

    #[tokio::test]
    async fn test_status_ok() {
        let tc = TestContextBuilder::new().build();
        status(tc.context(), "status message", None).await.unwrap();
    }
}
