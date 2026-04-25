//! Test harness for integration tests.
//!
//! Each test builds a small Pulumi program from `testdata/<name>/`, spins up a
//! local file-backend stack, runs `pulumi up`, asserts on outputs, then destroys.
//!
//! Gate tests with `--features integration` so they don't run during normal CI:
//!
//! ```text
//! cargo test -p integration-tests --features integration
//! ```

use std::path::{Path, PathBuf};

use pulumi_automation::{ConfigValue, LocalWorkspace, Stack, UpResult};
use tempfile::TempDir;

const PKG_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// Returns the path to `integration-tests/testdata/<name>`.
fn testdata_dir(name: &str) -> PathBuf {
    Path::new(PKG_DIR).join("testdata").join(name)
}

/// Builds the Pulumi program binary for the given testdata project.
///
/// Runs `cargo build -p pulumi-test-<name>` from the workspace root.  The
/// testdata programs are workspace members so they share the workspace
/// `target/` directory — no separate `protoc` invocation required.
async fn build_program(name: &str) -> anyhow::Result<()> {
    let package = format!("pulumi-test-{name}");
    let workspace_root = Path::new(PKG_DIR).parent().unwrap();
    let status = tokio::process::Command::new("cargo")
        .args(["build", "-p", &package])
        .current_dir(workspace_root)
        .status()
        .await?;
    anyhow::ensure!(status.success(), "cargo build -p {} failed", package);
    Ok(())
}

/// Drives a single Pulumi integration test end-to-end.
pub struct TestHarness {
    pub stack: Stack,
    _state_dir: TempDir,
}

impl TestHarness {
    /// Builds the program under `testdata/<name>/` and initialises a throw-away
    /// local-backend stack named `dev`.
    pub async fn setup(name: &str) -> anyhow::Result<Self> {
        build_program(name).await?;

        let state_dir = tempfile::tempdir()?;
        let workspace = LocalWorkspace::new(testdata_dir(name))
            .with_env(
                "PULUMI_BACKEND_URL",
                format!("file://{}", state_dir.path().display()),
            )
            .with_env("PULUMI_CONFIG_PASSPHRASE", "test")
            .with_env("PULUMI_SKIP_UPDATE_CHECK", "true");

        let stack = workspace.create_or_select_stack("dev").await?;

        Ok(TestHarness {
            stack,
            _state_dir: state_dir,
        })
    }

    /// Runs `pulumi up` and returns the result.
    pub async fn up(&self) -> pulumi_automation::Result<UpResult> {
        self.stack.up().await
    }

    /// Runs `pulumi destroy`, removes the stack record, and cleans up the
    /// temporary state directory.
    pub async fn destroy_and_cleanup(self) -> pulumi_automation::Result<()> {
        let ws = self.stack.workspace().clone();
        let stack_name = self.stack.name().to_string();
        self.stack.destroy().await?;
        ws.remove_stack(&stack_name, false).await?;
        Ok(())
        // _state_dir is dropped here, cleaning up the temp directory
    }

    /// Convenience: set a plain-text config value before running `up`.
    pub async fn set_config(&self, key: &str, value: &str) -> pulumi_automation::Result<()> {
        self.stack
            .set_config(key, &ConfigValue::plaintext(value))
            .await
    }

    /// Convenience: set a secret config value before running `up`.
    pub async fn set_secret_config(&self, key: &str, value: &str) -> pulumi_automation::Result<()> {
        self.stack
            .set_config(key, &ConfigValue::secret(value))
            .await
    }
}

/// Returns `true` if the `pulumi` binary is reachable on `$PATH`.
///
/// Integration tests call this at the start and return early (skipping rather
/// than failing) when `pulumi` is not installed.
pub fn pulumi_available() -> bool {
    std::process::Command::new("pulumi")
        .arg("version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
