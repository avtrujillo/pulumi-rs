//! Integration test: read config values inside a Pulumi program and export them.

#[tokio::test]
async fn test_config() {
    if !integration_tests::pulumi_available() {
        eprintln!("SKIP: pulumi binary not found on PATH");
        return;
    }

    let harness = integration_tests::TestHarness::setup("config")
        .await
        .expect("harness setup failed");

    // Set a config value that the program will read.
    // Pulumi namespaces config keys as `<project>:<key>`; the project name is
    // `config` (matching Pulumi.yaml). The program uses the full namespaced key.
    harness
        .set_config("config:message", "hello-world")
        .await
        .expect("set_config failed");

    let result = harness.up().await.expect("pulumi up failed");

    let outputs = &result.outputs;
    assert!(
        outputs.contains_key("message"),
        "expected 'message' in stack outputs; got: {:?}",
        outputs.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        outputs["message"].value,
        serde_json::json!("hello-world"),
        "output 'message' should equal the config value"
    );

    harness
        .destroy_and_cleanup()
        .await
        .expect("cleanup failed");
}
