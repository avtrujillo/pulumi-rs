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
async fn log_message(
    ctx: &Context,
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

    let mut engine = ctx.engine().await;
    engine.log(req).await?;

    Ok(())
}

/// Logs a debug message.
pub async fn debug(ctx: &Context, message: &str, urn: Option<&str>) -> Result<()> {
    log_message(ctx, Severity::Debug, message, urn, false).await
}

/// Logs an informational message.
pub async fn info(ctx: &Context, message: &str, urn: Option<&str>) -> Result<()> {
    log_message(ctx, Severity::Info, message, urn, false).await
}

/// Logs a warning message.
pub async fn warn(ctx: &Context, message: &str, urn: Option<&str>) -> Result<()> {
    log_message(ctx, Severity::Warning, message, urn, false).await
}

/// Logs an error message.
pub async fn error(ctx: &Context, message: &str, urn: Option<&str>) -> Result<()> {
    log_message(ctx, Severity::Error, message, urn, false).await
}

/// Logs an ephemeral status message (shown during operations but not persisted).
pub async fn status(ctx: &Context, message: &str, urn: Option<&str>) -> Result<()> {
    log_message(ctx, Severity::Info, message, urn, true).await
}
