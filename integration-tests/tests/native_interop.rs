//! Round-trip interop between `NativeStack`'s `Pulumi.<stack>.yaml` secret
//! handling and the real Pulumi CLI.
//!
//! The CLI and the native stack read and write the same stack file with the
//! same passphrase: a secret written by either side must decrypt on the
//! other. This is the end-to-end proof that the passphrase provider's wire
//! format (PBKDF2 key derivation, AES-GCM ciphertext, salt state, validation
//! token) matches the Go implementation.

use integration_tests::pulumi_available;
use pulumi_automation::{ConfigValue, LocalWorkspace};

const PASSPHRASE: &str = "test-passphrase";

#[tokio::test]
async fn native_stack_config_interops_with_cli() -> anyhow::Result<()> {
    if !pulumi_available() {
        eprintln!("skipping native_stack_config_interops_with_cli: pulumi not on PATH");
        return Ok(());
    }

    let project_dir = tempfile::tempdir()?;
    let state_dir = tempfile::tempdir()?;
    std::fs::write(
        project_dir.path().join("Pulumi.yaml"),
        "name: native-interop\nruntime: yaml\n",
    )?;

    let ws = LocalWorkspace::new(project_dir.path())
        .with_env(
            "PULUMI_BACKEND_URL",
            format!("file://{}", state_dir.path().display()),
        )
        .with_env("PULUMI_CONFIG_PASSPHRASE", PASSPHRASE)
        .with_env("PULUMI_SKIP_UPDATE_CHECK", "true");

    let cli_stack = ws.create_or_select_stack("dev").await?;
    let native = ws.native_stack("dev", vec![]).with_passphrase(PASSPHRASE);

    // CLI writes a secret; the native stack must restore the CLI's salt
    // (passphrase validation included) and decrypt the CLI's ciphertext.
    cli_stack
        .set_config(
            "native-interop:cliSecret",
            &ConfigValue::secret("s3cr3t-from-cli"),
        )
        .await?;
    let cv = native.get_config("native-interop:cliSecret")?;
    assert_eq!(cv.value, "s3cr3t-from-cli");
    assert!(cv.is_secret);

    // The native stack writes a secret with the same salt; the CLI must
    // decrypt it.
    native.set_config(
        "native-interop:rustSecret",
        ConfigValue::secret("s3cr3t-from-rust"),
    )?;
    let cli_read = cli_stack.get_config("native-interop:rustSecret").await?;
    assert_eq!(cli_read.value, "s3cr3t-from-rust");
    assert!(cli_read.is_secret);

    // The native save rewrites the whole stack file (re-encrypting every
    // secret), so the CLI's original secret must still decrypt afterwards.
    let cli_reread = cli_stack.get_config("native-interop:cliSecret").await?;
    assert_eq!(cli_reread.value, "s3cr3t-from-cli");

    // Plain values written natively read back through the CLI unencrypted.
    native.set_config(
        "native-interop:region",
        ConfigValue::plaintext("us-east-1"),
    )?;
    let plain = cli_stack.get_config("native-interop:region").await?;
    assert_eq!(plain.value, "us-east-1");
    assert!(!plain.is_secret);

    ws.remove_stack("dev", true).await?;
    Ok(())
}
