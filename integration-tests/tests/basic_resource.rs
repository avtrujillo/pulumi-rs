//! Integration test: create a `random:RandomString` resource and verify outputs.

#[tokio::test]
async fn test_basic_resource() {
    if !integration_tests::pulumi_available() {
        eprintln!("SKIP: pulumi binary not found on PATH");
        return;
    }

    let harness = integration_tests::TestHarness::setup("basic-resource")
        .await
        .expect("harness setup failed");

    let result = harness.up().await.expect("pulumi up failed");

    // The program exports `result` (the random string) and `length` (16).
    let outputs = &result.outputs;
    assert!(
        outputs.contains_key("result"),
        "expected 'result' in stack outputs; got: {:?}",
        outputs.keys().collect::<Vec<_>>()
    );
    assert!(
        outputs.contains_key("length"),
        "expected 'length' in stack outputs"
    );

    let length_val = &outputs["length"].value;
    assert_eq!(
        *length_val,
        serde_json::json!(16),
        "expected length output to be 16"
    );

    let result_val = &outputs["result"].value;
    assert!(
        result_val.is_string(),
        "expected result output to be a string, got: {result_val:?}"
    );
    let s = result_val.as_str().unwrap();
    assert!(!s.is_empty(), "result string should not be empty");
    assert_eq!(s.len(), 16, "result string should be 16 characters");

    harness
        .destroy_and_cleanup()
        .await
        .expect("cleanup failed");
}
