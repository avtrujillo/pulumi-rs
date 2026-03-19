//! Low-level interface for invoking the Pulumi CLI as a subprocess.

use crate::error::{Error, Result};
use std::path::Path;
use tokio::process::Command;

/// Output from a CLI invocation.
#[derive(Debug)]
pub(crate) struct CmdOutput {
    pub stdout: String,
    pub stderr: String,
}

/// Run `pulumi <args>` in the given working directory.
///
/// Returns the captured stdout/stderr on success, or an [`Error::CommandFailed`]
/// if the process exits with a non-zero status.
pub(crate) async fn run_pulumi_cmd(
    work_dir: &Path,
    args: &[&str],
    env: &[(&str, &str)],
) -> Result<CmdOutput> {
    let mut cmd = Command::new("pulumi");
    cmd.args(args)
        .current_dir(work_dir)
        // Disable interactive prompts and color codes.
        .env("PULUMI_NON_INTERACTIVE", "true")
        .env("NO_COLOR", "1");

    for &(k, v) in env {
        cmd.env(k, v);
    }

    let output = cmd.output().await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::CliNotFound(e)
        } else {
            Error::Io(e)
        }
    })?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    if !output.status.success() {
        return Err(Error::CommandFailed {
            command: args.join(" "),
            code: output.status.code().unwrap_or(-1),
            stdout,
            stderr,
        });
    }

    Ok(CmdOutput { stdout, stderr })
}
