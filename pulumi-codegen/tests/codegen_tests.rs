use pulumi_codegen::naming::{
    camel_to_snake_case, escape_rust_keyword, module_to_rust_identifier, parse_type_token,
    to_pascal_case, type_name_to_file_name,
};
use pulumi_codegen::schema::PackageSchema;

// ---- Schema parsing integration tests against real pulumi-random schema ----

fn load_random_schema() -> PackageSchema {
    let json = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/random_schema.json"
    ))
    .expect("Failed to read random_schema.json");
    serde_json::from_str(&json).expect("Failed to parse random_schema.json")
}

#[test]
fn random_schema_basic_metadata() {
    let schema = load_random_schema();
    assert_eq!(schema.name, "random");
    assert!(schema.meta.is_some());
    assert_eq!(
        schema.meta.as_ref().unwrap().module_format.as_deref(),
        Some("(.*)(?:/[^/]*)")
    );
}

#[test]
fn random_schema_has_expected_resources() {
    let schema = load_random_schema();

    let expected_resources = [
        "random:index/randomBytes:RandomBytes",
        "random:index/randomId:RandomId",
        "random:index/randomInteger:RandomInteger",
        "random:index/randomPassword:RandomPassword",
        "random:index/randomPet:RandomPet",
        "random:index/randomShuffle:RandomShuffle",
        "random:index/randomString:RandomString",
        "random:index/randomUuid:RandomUuid",
    ];

    for token in &expected_resources {
        assert!(
            schema.resources.contains_key(*token),
            "Missing resource: {token}"
        );
    }
}

#[test]
fn random_schema_random_id_resource() {
    let schema = load_random_schema();
    let random_id = &schema.resources["random:index/randomId:RandomId"];

    // Check input properties exist
    assert!(random_id.input_properties.contains_key("byteLength"));
    assert!(random_id.input_properties.contains_key("keepers"));
    assert!(random_id.input_properties.contains_key("prefix"));

    // Check output properties exist
    assert!(random_id.properties.contains_key("b64Std"));
    assert!(random_id.properties.contains_key("b64Url"));
    assert!(random_id.properties.contains_key("hex"));
    assert!(random_id.properties.contains_key("dec"));

    // Check required fields
    assert!(random_id.required_inputs.contains(&"byteLength".to_string()));
    assert!(random_id.required.contains(&"b64Std".to_string()));
    assert!(random_id.required.contains(&"hex".to_string()));

    // Check property types
    let byte_length = &random_id.input_properties["byteLength"];
    assert_eq!(byte_length.type_.as_deref(), Some("integer"));

    let keepers = &random_id.input_properties["keepers"];
    assert_eq!(keepers.type_.as_deref(), Some("object"));
    assert!(keepers.additional_properties.is_some());

    // Not a component
    assert!(!random_id.is_component);
    assert!(!random_id.is_overlay);
}

#[test]
fn random_schema_random_string_resource() {
    let schema = load_random_schema();
    let random_string = &schema.resources["random:index/randomString:RandomString"];

    // RandomString has many input properties for character set control
    assert!(random_string.input_properties.contains_key("length"));
    assert!(random_string.input_properties.contains_key("special"));
    assert!(random_string.input_properties.contains_key("upper"));
    assert!(random_string.input_properties.contains_key("lower"));
    assert!(random_string.input_properties.contains_key("numeric"));

    // length is required
    assert!(random_string
        .required_inputs
        .contains(&"length".to_string()));

    let length = &random_string.input_properties["length"];
    assert_eq!(length.type_.as_deref(), Some("integer"));

    let special = &random_string.input_properties["special"];
    assert_eq!(special.type_.as_deref(), Some("boolean"));
}

#[test]
fn random_schema_token_parsing() {
    let schema = load_random_schema();
    let module_format = schema
        .meta
        .as_ref()
        .and_then(|m| m.module_format.as_deref());

    // All random resources should be in the index (root) module
    for token in schema.resources.keys() {
        let parsed = parse_type_token(token, module_format).unwrap();
        assert_eq!(parsed.package, "random");
        assert_eq!(parsed.module, "", "Expected index module for {token}");
    }
}

// ---- Naming integration tests: end-to-end pipeline from token to Rust names ----

#[test]
fn naming_pipeline_random_id() {
    let token = "random:index/randomId:RandomId";
    let parsed = parse_type_token(token, Some("(.*)(?:/[^/]*)")).unwrap();

    assert_eq!(parsed.package, "random");
    assert_eq!(parsed.module, "");
    assert_eq!(parsed.name, "RandomId");

    // Type name stays PascalCase
    assert_eq!(to_pascal_case(&parsed.name), "RandomId");

    // File name is snake_case
    assert_eq!(type_name_to_file_name(&parsed.name), "random_id");

    // Field name conversion
    assert_eq!(camel_to_snake_case("byteLength"), "byte_length");
    assert_eq!(camel_to_snake_case("b64Std"), "b64_std");
    assert_eq!(camel_to_snake_case("b64Url"), "b64_url");
}

#[test]
fn naming_pipeline_aws_style() {
    let token = "aws:s3/bucket:Bucket";
    let parsed = parse_type_token(token, Some("(.*)(?:/[^/]*)")).unwrap();

    assert_eq!(parsed.package, "aws");
    assert_eq!(parsed.module, "s3");
    assert_eq!(parsed.name, "Bucket");

    assert_eq!(module_to_rust_identifier(&parsed.module), "s3");
    assert_eq!(type_name_to_file_name(&parsed.name), "bucket");
}

#[test]
fn naming_pipeline_keyword_field() {
    // Simulate a property named "type" (common in Pulumi schemas)
    let snake = camel_to_snake_case("type");
    let escaped = escape_rust_keyword(&snake);
    assert_eq!(escaped, "r#type");

    // "self" field
    let escaped = escape_rust_keyword("self");
    assert_eq!(escaped, "self_");
}
