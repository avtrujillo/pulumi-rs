//! Integration test: create a `random:RandomPassword` resource.
//!
//! This exercises the secrets subsystem: RandomPassword marks its `result`
//! output as secret, so the file-backend must encrypt it with the passphrase.
//! A successful `up` + `destroy` proves the encrypt/decrypt round-trip works.

#[tokio::test]
async fn test_secrets() {
    if !integration_tests::pulumi_available() {
        eprintln!("SKIP: pulumi binary not found on PATH");
        return;
    }

    let harness = integration_tests::TestHarness::setup("secrets")
        .await
        .expect("harness setup failed");

    let result = harness.up().await.expect("pulumi up failed");

    // The program exports `length` as a plain output (not secret) so we can
    // assert on it without needing --show-secrets.
    let outputs = &result.outputs;
    assert!(
        outputs.contains_key("length"),
        "expected 'length' in stack outputs; got: {:?}",
        outputs.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        outputs["length"].value,
        serde_json::json!(20),
        "expected length output to be 20"
    );

    // `result` is marked secret by the random provider; the output value is
    // masked as "[secret]" by the CLI unless --show-secrets is passed.
    assert!(
        outputs.contains_key("result"),
        "expected 'result' in stack outputs"
    );
    assert!(
        outputs["result"].secret,
        "expected 'result' output to be marked as secret"
    );

    harness
        .destroy_and_cleanup()
        .await
        .expect("cleanup failed");
}
