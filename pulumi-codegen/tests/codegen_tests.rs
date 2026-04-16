use pulumi_codegen::emit::emit_package;
use pulumi_codegen::ir::{resolve_package, ResolvedType};
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
        "random:index/randomUuid4:RandomUuid4",
        "random:index/randomUuid7:RandomUuid7",
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

// ---- IR integration tests: resolve_package against real pulumi-random schema ----

#[test]
fn random_schema_ir_basic_structure() {
    let schema = load_random_schema();
    let package = resolve_package(&schema);

    assert_eq!(package.name, "random");
    // random provider has no top-level version in the fixture
    assert_eq!(package.version, "0.0.0");

    // All resources are in the root module; there's also a "providers" module
    // from the provider method function `pulumi:providers:random/terraformConfig`.
    assert!(package.modules.contains_key(""));
    assert!(package.modules.contains_key("providers"));

    let root = &package.modules[""];
    assert_eq!(root.resources.len(), 10);
    assert_eq!(root.functions.len(), 0);

    // The providers module has the terraformConfig method function
    let providers = &package.modules["providers"];
    assert_eq!(providers.resources.len(), 0);
    assert_eq!(providers.functions.len(), 1);
    // Token `pulumi:providers:random/terraformConfig` — the name segment
    // is `random/terraformConfig`, which to_pascal_case produces as-is with
    // the slash preserved since it's not a delimiter in our PascalCase logic.
    assert_eq!(providers.functions.len(), 1);

    // random provider has no complex types
    assert!(package.types.is_empty());
}

#[test]
fn random_schema_ir_random_id_resource() {
    let schema = load_random_schema();
    let package = resolve_package(&schema);
    let root = &package.modules[""];

    let random_id = root
        .resources
        .iter()
        .find(|r| r.rust_name == "RandomId")
        .expect("RandomId resource not found");

    assert_eq!(random_id.type_token, "random:index/randomId:RandomId");
    assert_eq!(random_id.file_name, "random_id");
    assert!(!random_id.is_component);

    // Input fields
    let byte_length = random_id
        .input_fields
        .iter()
        .find(|f| f.rust_name == "byte_length")
        .expect("byte_length input not found");
    assert_eq!(byte_length.rust_type, "i64");
    assert!(byte_length.required);

    let keepers = random_id
        .input_fields
        .iter()
        .find(|f| f.rust_name == "keepers")
        .expect("keepers input not found");
    assert_eq!(
        keepers.rust_type,
        "Option<std::collections::HashMap<String, String>>"
    );
    assert!(!keepers.required);

    let prefix = random_id
        .input_fields
        .iter()
        .find(|f| f.rust_name == "prefix")
        .expect("prefix input not found");
    assert_eq!(prefix.rust_type, "Option<String>");

    // Output fields
    let b64_std = random_id
        .output_fields
        .iter()
        .find(|f| f.rust_name == "b64_std")
        .expect("b64_std output not found");
    assert_eq!(b64_std.rust_type, "String");
    assert!(b64_std.required);
    assert_eq!(b64_std.original_name, "b64Std");

    let hex = random_id
        .output_fields
        .iter()
        .find(|f| f.rust_name == "hex")
        .expect("hex output not found");
    assert_eq!(hex.rust_type, "String");

    let dec = random_id
        .output_fields
        .iter()
        .find(|f| f.rust_name == "dec")
        .expect("dec output not found");
    assert_eq!(dec.rust_type, "String");
}

#[test]
fn random_schema_ir_all_resources_named() {
    let schema = load_random_schema();
    let package = resolve_package(&schema);
    let root = &package.modules[""];

    let expected: Vec<(&str, &str)> = vec![
        ("RandomBytes", "random_bytes"),
        ("RandomId", "random_id"),
        ("RandomInteger", "random_integer"),
        ("RandomPassword", "random_password"),
        ("RandomPet", "random_pet"),
        ("RandomShuffle", "random_shuffle"),
        ("RandomString", "random_string"),
        ("RandomUuid", "random_uuid"),
        ("RandomUuid4", "random_uuid4"),
        ("RandomUuid7", "random_uuid7"),
    ];

    for (rust_name, file_name) in &expected {
        let res = root
            .resources
            .iter()
            .find(|r| r.rust_name == *rust_name)
            .unwrap_or_else(|| panic!("Missing resource: {rust_name}"));
        assert_eq!(res.file_name, *file_name, "Wrong file_name for {rust_name}");
    }
}

#[test]
fn synthetic_schema_with_types_and_functions() {
    let json = r##"{
        "name": "mycloud",
        "version": "1.2.3",
        "meta": { "moduleFormat": "(.*)(?:/[^/]*)" },
        "resources": {
            "mycloud:storage/bucket:Bucket": {
                "description": "A storage bucket",
                "inputProperties": {
                    "name": { "type": "string" },
                    "rules": {
                        "type": "array",
                        "items": { "$ref": "#/types/mycloud:storage/BucketRule:BucketRule" }
                    },
                    "acl": {
                        "$ref": "#/types/mycloud:storage/CannedAcl:CannedAcl"
                    }
                },
                "properties": {
                    "id": { "type": "string" },
                    "name": { "type": "string" },
                    "arn": { "type": "string" }
                },
                "requiredInputs": ["name"],
                "required": ["id", "name", "arn"]
            }
        },
        "functions": {
            "mycloud:storage/getBucket:getBucket": {
                "description": "Look up a bucket",
                "inputs": {
                    "properties": {
                        "name": { "type": "string" }
                    },
                    "required": ["name"]
                },
                "outputs": {
                    "properties": {
                        "id": { "type": "string" },
                        "arn": { "type": "string" }
                    },
                    "required": ["id", "arn"]
                }
            }
        },
        "types": {
            "mycloud:storage/BucketRule:BucketRule": {
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean" },
                    "prefix": { "type": "string" }
                },
                "required": ["enabled"]
            },
            "mycloud:storage/CannedAcl:CannedAcl": {
                "type": "string",
                "enum": [
                    { "name": "Private", "value": "private" },
                    { "name": "PublicRead", "value": "public-read" },
                    { "value": "authenticated-read" }
                ]
            }
        }
    }"##;

    let schema: PackageSchema = serde_json::from_str(json).unwrap();
    let package = resolve_package(&schema);

    assert_eq!(package.name, "mycloud");
    assert_eq!(package.version, "1.2.3");

    // One module: "storage"
    assert_eq!(package.modules.len(), 1);
    assert!(package.modules.contains_key("storage"));

    let storage = &package.modules["storage"];

    // Resource
    assert_eq!(storage.resources.len(), 1);
    let bucket = &storage.resources[0];
    assert_eq!(bucket.rust_name, "Bucket");
    assert_eq!(bucket.file_name, "bucket");

    // Resource field referencing an array of complex type
    let rules = bucket
        .input_fields
        .iter()
        .find(|f| f.rust_name == "rules")
        .unwrap();
    assert_eq!(rules.rust_type, "Option<Vec<crate::types::storage::BucketRule>>");

    // Resource field referencing an enum type
    let acl = bucket
        .input_fields
        .iter()
        .find(|f| f.rust_name == "acl")
        .unwrap();
    assert_eq!(acl.rust_type, "Option<crate::types::storage::CannedAcl>");

    // Function
    assert_eq!(storage.functions.len(), 1);
    let get_bucket = &storage.functions[0];
    assert_eq!(get_bucket.rust_name, "GetBucket");
    assert_eq!(get_bucket.file_name, "get_bucket");
    assert_eq!(get_bucket.arg_fields.len(), 1);
    assert_eq!(get_bucket.arg_fields[0].rust_name, "name");
    assert_eq!(get_bucket.arg_fields[0].rust_type, "String");
    assert_eq!(get_bucket.result_fields.len(), 2);

    // Types
    assert_eq!(package.types.len(), 2);

    // Object type
    let rule_type = &package.types["mycloud:storage/BucketRule:BucketRule"];
    match rule_type {
        ResolvedType::Object(obj) => {
            assert_eq!(obj.rust_name, "BucketRule");
            assert_eq!(obj.module, "storage");
            assert_eq!(obj.fields.len(), 2);
            let enabled = obj.fields.iter().find(|f| f.rust_name == "enabled").unwrap();
            assert_eq!(enabled.rust_type, "bool");
            assert!(enabled.required);
            let prefix = obj.fields.iter().find(|f| f.rust_name == "prefix").unwrap();
            assert_eq!(prefix.rust_type, "Option<String>");
        }
        ResolvedType::Enum(_) => panic!("Expected object"),
    }

    // Enum type
    let acl_type = &package.types["mycloud:storage/CannedAcl:CannedAcl"];
    match acl_type {
        ResolvedType::Enum(e) => {
            assert_eq!(e.rust_name, "CannedAcl");
            assert_eq!(e.module, "storage");
            assert_eq!(e.underlying_type, "String");
            assert_eq!(e.variants.len(), 3);
            assert_eq!(e.variants[0].rust_name, "Private");
            assert_eq!(e.variants[0].value, "private");
            assert_eq!(e.variants[1].rust_name, "PublicRead");
            assert_eq!(e.variants[1].value, "public-read");
            // Unnamed variant derived from value
            assert_eq!(e.variants[2].rust_name, "AuthenticatedRead");
            assert_eq!(e.variants[2].value, "authenticated-read");
        }
        ResolvedType::Object(_) => panic!("Expected enum"),
    }
}

// ---- End-to-end: emit pulumi-random crate and verify file structure ----

#[test]
fn emit_random_crate_file_structure() {
    let schema = load_random_schema();
    let package = resolve_package(&schema);
    let dir = tempfile::tempdir().unwrap();

    emit_package(&package, dir.path()).unwrap();

    // Cargo.toml
    let cargo = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
    assert!(cargo.contains("name = \"pulumi-random\""));
    assert!(cargo.contains("pulumi = {"));

    // src/lib.rs should have mod + pub use for each resource
    let lib = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
    assert!(lib.contains("mod random_id;"));
    assert!(lib.contains("pub use random_id::*;"));
    assert!(lib.contains("mod random_string;"));
    assert!(lib.contains("pub use random_string::*;"));
    assert!(lib.contains("mod random_bytes;"));

    // Each resource file should exist and contain expected code
    let resource_files = [
        "random_bytes", "random_id", "random_integer", "random_password",
        "random_pet", "random_shuffle", "random_string", "random_uuid",
        "random_uuid4", "random_uuid7",
    ];
    for name in &resource_files {
        let path = dir.path().join(format!("src/{name}.rs"));
        assert!(path.exists(), "Missing resource file: {name}.rs");
    }

    // Spot-check random_id.rs content
    let random_id = std::fs::read_to_string(dir.path().join("src/random_id.rs")).unwrap();
    assert!(random_id.contains("pub struct RandomIdArgs {"));
    assert!(random_id.contains("pub struct RandomIdOutputs {"));
    assert!(random_id.contains("#[derive(pulumi::Resource)]"));
    assert!(random_id.contains("#[pulumi(type_token = \"random:index/randomId:RandomId\")]"));
    assert!(random_id.contains("#[pulumi(inputs = RandomIdArgs)]"));
    assert!(random_id.contains("#[pulumi(outputs = RandomIdOutputs)]"));
    assert!(random_id.contains("pub struct RandomId;"));

    // Check field attributes in RandomIdArgs
    assert!(random_id.contains("#[serde(rename = \"byteLength\")]"));
    assert!(random_id.contains("pub byte_length: i64,"));
    assert!(random_id.contains(
        "#[serde(rename = \"keepers\", skip_serializing_if = \"Option::is_none\")]"
    ));
    assert!(random_id.contains(
        "pub keepers: Option<std::collections::HashMap<String, String>>,"
    ));

    // Check outputs don't have skip_serializing_if
    assert!(random_id.contains("pub struct RandomIdOutputs {"));
    // b64Std is a required output — should just have rename, no skip_serializing_if
    assert!(random_id.contains("#[serde(rename = \"b64Std\")]"));
    assert!(random_id.contains("pub b64_std: String,"));
}

#[test]
fn emit_synthetic_crate_with_modules_and_types() {
    let json = r##"{
        "name": "mycloud",
        "version": "1.0.0",
        "meta": { "moduleFormat": "(.*)(?:/[^/]*)" },
        "resources": {
            "mycloud:storage/bucket:Bucket": {
                "inputProperties": {
                    "name": { "type": "string" },
                    "acl": { "$ref": "#/types/mycloud:storage/CannedAcl:CannedAcl" }
                },
                "properties": {
                    "id": { "type": "string" }
                },
                "requiredInputs": ["name"],
                "required": ["id"]
            }
        },
        "functions": {
            "mycloud:storage/getBucket:getBucket": {
                "inputs": {
                    "properties": { "name": { "type": "string" } },
                    "required": ["name"]
                },
                "outputs": {
                    "properties": { "id": { "type": "string" } },
                    "required": ["id"]
                }
            }
        },
        "types": {
            "mycloud:storage/BucketRule:BucketRule": {
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean" }
                },
                "required": ["enabled"]
            },
            "mycloud:storage/CannedAcl:CannedAcl": {
                "type": "string",
                "enum": [
                    { "name": "Private", "value": "private" },
                    { "name": "PublicRead", "value": "public-read" }
                ]
            }
        }
    }"##;

    let schema: PackageSchema = serde_json::from_str(json).unwrap();
    let package = resolve_package(&schema);
    let dir = tempfile::tempdir().unwrap();

    emit_package(&package, dir.path()).unwrap();

    // Module directory structure
    assert!(dir.path().join("src/storage/mod.rs").exists());
    assert!(dir.path().join("src/storage/bucket.rs").exists());
    assert!(dir.path().join("src/storage/get_bucket.rs").exists());

    // Types directory structure
    assert!(dir.path().join("src/types/mod.rs").exists());
    assert!(dir.path().join("src/types/storage/mod.rs").exists());
    assert!(dir.path().join("src/types/storage/bucket_rule.rs").exists());
    assert!(dir.path().join("src/types/storage/canned_acl.rs").exists());

    // lib.rs should declare the storage module and types module
    let lib = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
    assert!(lib.contains("pub mod types;"));
    assert!(lib.contains("pub mod storage;"));

    // storage/mod.rs should declare bucket and get_bucket
    let storage_mod = std::fs::read_to_string(dir.path().join("src/storage/mod.rs")).unwrap();
    assert!(storage_mod.contains("mod bucket;"));
    assert!(storage_mod.contains("pub use bucket::*;"));
    assert!(storage_mod.contains("mod get_bucket;"));
    assert!(storage_mod.contains("pub use get_bucket::*;"));

    // types/mod.rs should declare storage submodule
    let types_mod = std::fs::read_to_string(dir.path().join("src/types/mod.rs")).unwrap();
    assert!(types_mod.contains("pub mod storage;"));

    // types/storage/mod.rs should declare the types
    let types_storage = std::fs::read_to_string(dir.path().join("src/types/storage/mod.rs")).unwrap();
    assert!(types_storage.contains("mod bucket_rule;"));
    assert!(types_storage.contains("pub use bucket_rule::*;"));
    assert!(types_storage.contains("mod canned_acl;"));
    assert!(types_storage.contains("pub use canned_acl::*;"));

    // Object type file
    let rule = std::fs::read_to_string(dir.path().join("src/types/storage/bucket_rule.rs")).unwrap();
    assert!(rule.contains("#[derive(Serialize, Deserialize, Clone)]"));
    assert!(rule.contains("pub struct BucketRule {"));
    assert!(rule.contains("pub enabled: bool,"));

    // Enum type file
    let acl = std::fs::read_to_string(dir.path().join("src/types/storage/canned_acl.rs")).unwrap();
    assert!(acl.contains("#[derive(Serialize, Deserialize, Clone, PartialEq)]"));
    assert!(acl.contains("pub enum CannedAcl {"));
    assert!(acl.contains("#[serde(rename = \"private\")]"));
    assert!(acl.contains("    Private,"));
    assert!(acl.contains("#[serde(rename = \"public-read\")]"));
    assert!(acl.contains("    PublicRead,"));

    // Function file
    let get_bucket = std::fs::read_to_string(dir.path().join("src/storage/get_bucket.rs")).unwrap();
    assert!(get_bucket.contains("pub struct GetBucketArgs {"));
    assert!(get_bucket.contains("pub struct GetBucketResult {"));
    assert!(get_bucket.contains("#[derive(pulumi::ProviderFunction)]"));
    assert!(get_bucket.contains("pub struct GetBucket;"));
}
