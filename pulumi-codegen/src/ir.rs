//! Intermediate representation for resolved Pulumi provider schemas.
//!
//! Transforms a parsed [`PackageSchema`] into a fully resolved IR where every
//! type token has been parsed, every `$ref` followed, and every property mapped
//! to a concrete Rust type string. The IR is organized by module and ready for
//! direct code emission.

use std::collections::BTreeMap;

use crate::naming::{
    camel_to_snake_case, escape_rust_keyword, module_to_rust_identifier, parse_type_token,
    to_pascal_case, type_name_to_file_name,
};
use crate::schema::{
    ComplexTypeSpec, FunctionSpec, PackageSchema, PropertySpec, ResourceSpec, TypeSpec,
};

// ---------------------------------------------------------------------------
// IR type definitions
// ---------------------------------------------------------------------------

/// A fully resolved provider package, ready for code emission.
#[derive(Debug)]
pub struct ResolvedPackage {
    pub name: String,
    pub version: String,
    /// Modules keyed by Rust module identifier. `""` = crate root (index module).
    pub modules: BTreeMap<String, ResolvedModule>,
    /// Complex types keyed by original schema token.
    pub types: BTreeMap<String, ResolvedType>,
}

/// A module containing resolved resources and functions.
#[derive(Debug, Default)]
pub struct ResolvedModule {
    pub resources: Vec<ResolvedResource>,
    pub functions: Vec<ResolvedFunction>,
}

/// A resolved resource definition.
#[derive(Debug)]
pub struct ResolvedResource {
    pub type_token: String,
    pub rust_name: String,
    pub file_name: String,
    pub description: Option<String>,
    pub deprecation: Option<String>,
    pub is_component: bool,
    pub input_fields: Vec<ResolvedField>,
    pub output_fields: Vec<ResolvedField>,
}

/// A resolved function (invoke) definition.
#[derive(Debug)]
pub struct ResolvedFunction {
    pub token: String,
    pub rust_name: String,
    pub file_name: String,
    pub description: Option<String>,
    pub deprecation: Option<String>,
    pub arg_fields: Vec<ResolvedField>,
    pub result_fields: Vec<ResolvedField>,
}

/// A resolved field (property) on a resource, function, or complex type.
#[derive(Debug)]
pub struct ResolvedField {
    /// Original wire name (for `#[serde(rename)]`).
    pub original_name: String,
    /// Rust field name (snake_case, keyword-escaped).
    pub rust_name: String,
    /// Full Rust type string (e.g. `"Option<Vec<String>>"`).
    pub rust_type: String,
    pub description: Option<String>,
    pub deprecation: Option<String>,
    pub secret: bool,
    pub required: bool,
}

/// A resolved complex type — either an object or an enum.
#[derive(Debug)]
pub enum ResolvedType {
    Object(ResolvedObject),
    Enum(ResolvedEnum),
}

/// A resolved object (struct) type.
#[derive(Debug)]
pub struct ResolvedObject {
    pub token: String,
    pub rust_name: String,
    /// Rust module identifier (`""` for root).
    pub module: String,
    pub file_name: String,
    pub description: Option<String>,
    pub fields: Vec<ResolvedField>,
}

/// A resolved enum type.
#[derive(Debug)]
pub struct ResolvedEnum {
    pub token: String,
    pub rust_name: String,
    /// Rust module identifier (`""` for root).
    pub module: String,
    pub file_name: String,
    pub description: Option<String>,
    /// Rust type of the underlying value (e.g. `"String"`, `"i64"`).
    pub underlying_type: String,
    pub variants: Vec<ResolvedEnumVariant>,
}

/// A single variant of a resolved enum.
#[derive(Debug)]
pub struct ResolvedEnumVariant {
    pub rust_name: String,
    pub value: String,
    pub description: Option<String>,
    pub deprecation: Option<String>,
}

// ---------------------------------------------------------------------------
// Type resolution
// ---------------------------------------------------------------------------

/// Resolve a `$ref` string to a Rust type path.
///
/// Handles Pulumi built-in refs (`pulumi.json#/Any`, etc.) and local type
/// refs (`#/types/pkg:mod:Name`).
fn resolve_type_ref(ref_str: &str, module_format: Option<&str>) -> String {
    if ref_str.starts_with("pulumi.json#/") {
        return "serde_json::Value".to_string();
    }

    if let Some(token) = ref_str.strip_prefix("#/types/") {
        if let Some(parsed) = parse_type_token(token, module_format) {
            let rust_name = to_pascal_case(&parsed.name);
            let rust_module = module_to_rust_identifier(&parsed.module);
            return if rust_module.is_empty() {
                format!("crate::types::{rust_name}")
            } else {
                format!("crate::types::{rust_module}::{rust_name}")
            };
        }
    }

    // Unrecognized ref format — fall back to opaque JSON value.
    "serde_json::Value".to_string()
}

/// Core type resolution from the five type-describing fields shared by
/// [`PropertySpec`] and [`TypeSpec`].
fn resolve_raw_type(
    type_: Option<&str>,
    ref_: Option<&str>,
    items: Option<&TypeSpec>,
    additional_properties: Option<&TypeSpec>,
    one_of: Option<&[TypeSpec]>,
    module_format: Option<&str>,
) -> String {
    // Union types → opaque value for now.
    if one_of.is_some() {
        return "serde_json::Value".to_string();
    }

    // Direct $ref.
    if let Some(r) = ref_ {
        return resolve_type_ref(r, module_format);
    }

    // Primitive / composite types.
    match type_ {
        Some("string") => "String".to_string(),
        Some("integer") => "i64".to_string(),
        Some("number") => "f64".to_string(),
        Some("boolean") => "bool".to_string(),
        Some("array") => {
            let inner = items
                .map(|i| resolve_type_spec(i, module_format))
                .unwrap_or_else(|| "serde_json::Value".to_string());
            format!("Vec<{inner}>")
        }
        Some("object") => {
            if let Some(ap) = additional_properties {
                let inner = resolve_type_spec(ap, module_format);
                format!("std::collections::HashMap<String, {inner}>")
            } else {
                // Bare `object` with no additionalProperties — treat as map of any.
                "std::collections::HashMap<String, serde_json::Value>".to_string()
            }
        }
        _ => "serde_json::Value".to_string(),
    }
}

/// Resolve a [`TypeSpec`] (used in `items`, `additionalProperties`, `oneOf`).
fn resolve_type_spec(spec: &TypeSpec, module_format: Option<&str>) -> String {
    resolve_raw_type(
        spec.type_.as_deref(),
        spec.ref_.as_deref(),
        spec.items.as_deref(),
        spec.additional_properties.as_deref(),
        spec.one_of.as_deref(),
        module_format,
    )
}

/// Resolve a [`PropertySpec`] to its base Rust type string (before optional wrapping).
fn resolve_property_type(prop: &PropertySpec, module_format: Option<&str>) -> String {
    resolve_raw_type(
        prop.type_.as_deref(),
        prop.ref_.as_deref(),
        prop.items.as_deref(),
        prop.additional_properties.as_deref(),
        prop.one_of.as_deref(),
        module_format,
    )
}

// ---------------------------------------------------------------------------
// Field resolution
// ---------------------------------------------------------------------------

/// Resolve a single schema property into a [`ResolvedField`].
fn resolve_field(
    name: &str,
    prop: &PropertySpec,
    is_required: bool,
    module_format: Option<&str>,
) -> ResolvedField {
    let snake = camel_to_snake_case(name);
    let rust_name = escape_rust_keyword(&snake);
    let base_type = resolve_property_type(prop, module_format);
    let rust_type = if is_required {
        base_type
    } else {
        format!("Option<{base_type}>")
    };

    ResolvedField {
        original_name: name.to_string(),
        rust_name,
        rust_type,
        description: prop.description.clone(),
        deprecation: prop.deprecation_message.clone(),
        secret: prop.secret,
        required: is_required,
    }
}

/// Resolve all properties in a map into a sorted vec of [`ResolvedField`]s.
fn resolve_fields(
    properties: &BTreeMap<String, PropertySpec>,
    required: &[String],
    module_format: Option<&str>,
) -> Vec<ResolvedField> {
    properties
        .iter()
        .map(|(name, prop)| {
            let is_required = required.contains(name);
            resolve_field(name, prop, is_required, module_format)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Resource resolution
// ---------------------------------------------------------------------------

/// Resolve a single resource. Returns `None` for overlays.
fn resolve_resource(
    token: &str,
    spec: &ResourceSpec,
    module_format: Option<&str>,
) -> Option<ResolvedResource> {
    if spec.is_overlay {
        return None;
    }
    let parsed = parse_type_token(token, module_format)?;
    let rust_name = to_pascal_case(&parsed.name);
    let file_name = type_name_to_file_name(&parsed.name);
    let input_fields = resolve_fields(&spec.input_properties, &spec.required_inputs, module_format);
    let output_fields = resolve_fields(&spec.properties, &spec.required, module_format);

    Some(ResolvedResource {
        type_token: token.to_string(),
        rust_name,
        file_name,
        description: spec.description.clone(),
        deprecation: spec.deprecation_message.clone(),
        is_component: spec.is_component,
        input_fields,
        output_fields,
    })
}

// ---------------------------------------------------------------------------
// Function resolution
// ---------------------------------------------------------------------------

/// Resolve a single function. Returns `None` for overlays.
fn resolve_function(
    token: &str,
    spec: &FunctionSpec,
    module_format: Option<&str>,
) -> Option<ResolvedFunction> {
    if spec.is_overlay {
        return None;
    }
    let parsed = parse_type_token(token, module_format)?;
    let rust_name = to_pascal_case(&parsed.name);
    let file_name = type_name_to_file_name(&parsed.name);

    let arg_fields = spec
        .inputs
        .as_ref()
        .map(|obj| resolve_fields(&obj.properties, &obj.required, module_format))
        .unwrap_or_default();

    let result_fields = spec
        .outputs
        .as_ref()
        .map(|obj| resolve_fields(&obj.properties, &obj.required, module_format))
        .unwrap_or_default();

    Some(ResolvedFunction {
        token: token.to_string(),
        rust_name,
        file_name,
        description: spec.description.clone(),
        deprecation: spec.deprecation_message.clone(),
        arg_fields,
        result_fields,
    })
}

// ---------------------------------------------------------------------------
// Complex type resolution
// ---------------------------------------------------------------------------

/// Resolve a complex type (object or enum). Returns `None` for overlays.
fn resolve_complex_type(
    token: &str,
    spec: &ComplexTypeSpec,
    module_format: Option<&str>,
) -> Option<ResolvedType> {
    if spec.is_overlay {
        return None;
    }
    let parsed = parse_type_token(token, module_format)?;
    let rust_name = to_pascal_case(&parsed.name);
    let module = module_to_rust_identifier(&parsed.module);
    let file_name = type_name_to_file_name(&parsed.name);

    if spec.is_enum() {
        let underlying_type = match spec.type_.as_deref() {
            Some("integer") => "i64".to_string(),
            Some("number") => "f64".to_string(),
            _ => "String".to_string(),
        };

        let variants = spec
            .enum_values
            .as_ref()
            .unwrap()
            .iter()
            .map(|ev| {
                let variant_rust_name = if let Some(name) = &ev.name {
                    to_pascal_case(name)
                } else {
                    match &ev.value {
                        serde_json::Value::String(s) => to_pascal_case(s),
                        serde_json::Value::Number(n) => format!("V{n}"),
                        other => to_pascal_case(&other.to_string()),
                    }
                };
                let value_string = match &ev.value {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                ResolvedEnumVariant {
                    rust_name: variant_rust_name,
                    value: value_string,
                    description: ev.description.clone(),
                    deprecation: ev.deprecation_message.clone(),
                }
            })
            .collect();

        Some(ResolvedType::Enum(ResolvedEnum {
            token: token.to_string(),
            rust_name,
            module,
            file_name,
            description: spec.description.clone(),
            underlying_type,
            variants,
        }))
    } else {
        let fields = resolve_fields(&spec.properties, &spec.required, module_format);
        Some(ResolvedType::Object(ResolvedObject {
            token: token.to_string(),
            rust_name,
            module,
            file_name,
            description: spec.description.clone(),
            fields,
        }))
    }
}

// ---------------------------------------------------------------------------
// Top-level resolver
// ---------------------------------------------------------------------------

/// Resolve a parsed [`PackageSchema`] into a [`ResolvedPackage`].
///
/// This is the main entry point for IR construction. It parses every type
/// token, follows `$ref` references, maps types to Rust, and organizes
/// everything by module.
pub fn resolve_package(schema: &PackageSchema) -> ResolvedPackage {
    let module_format = schema.meta.as_ref().and_then(|m| m.module_format.as_deref());
    let version = schema.version.clone().unwrap_or_else(|| "0.0.0".to_string());

    // Resolve complex types.
    let types: BTreeMap<String, ResolvedType> = schema
        .types
        .iter()
        .filter_map(|(token, spec)| {
            resolve_complex_type(token, spec, module_format).map(|t| (token.clone(), t))
        })
        .collect();

    // Resolve resources, grouped by module.
    let mut module_resources: BTreeMap<String, Vec<ResolvedResource>> = BTreeMap::new();
    for (token, spec) in &schema.resources {
        if let Some(resource) = resolve_resource(token, spec, module_format) {
            let parsed = parse_type_token(token, module_format);
            let module_key = parsed
                .map(|p| module_to_rust_identifier(&p.module))
                .unwrap_or_default();
            module_resources.entry(module_key).or_default().push(resource);
        }
    }

    // Resolve functions, grouped by module.
    let mut module_functions: BTreeMap<String, Vec<ResolvedFunction>> = BTreeMap::new();
    for (token, spec) in &schema.functions {
        if let Some(function) = resolve_function(token, spec, module_format) {
            let parsed = parse_type_token(token, module_format);
            let module_key = parsed
                .map(|p| module_to_rust_identifier(&p.module))
                .unwrap_or_default();
            module_functions
                .entry(module_key)
                .or_default()
                .push(function);
        }
    }

    // Merge into modules.
    let mut modules: BTreeMap<String, ResolvedModule> = BTreeMap::new();
    for key in module_resources
        .keys()
        .chain(module_functions.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
    {
        modules.insert(
            key.clone(),
            ResolvedModule {
                resources: module_resources.remove(&key).unwrap_or_default(),
                functions: module_functions.remove(&key).unwrap_or_default(),
            },
        );
    }

    ResolvedPackage {
        name: schema.name.clone(),
        version,
        modules,
        types,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::TypeSpec;

    // ---- Type resolution: built-in refs ----

    #[test]
    fn resolve_builtin_any() {
        assert_eq!(
            resolve_type_ref("pulumi.json#/Any", None),
            "serde_json::Value"
        );
    }

    #[test]
    fn resolve_builtin_asset() {
        assert_eq!(
            resolve_type_ref("pulumi.json#/Asset", None),
            "serde_json::Value"
        );
    }

    #[test]
    fn resolve_builtin_archive() {
        assert_eq!(
            resolve_type_ref("pulumi.json#/Archive", None),
            "serde_json::Value"
        );
    }

    #[test]
    fn resolve_builtin_json() {
        assert_eq!(
            resolve_type_ref("pulumi.json#/Json", None),
            "serde_json::Value"
        );
    }

    // ---- Type resolution: local $ref ----

    #[test]
    fn resolve_ref_index_type() {
        assert_eq!(
            resolve_type_ref("#/types/random:index/thing:Thing", None),
            "crate::types::Thing"
        );
    }

    #[test]
    fn resolve_ref_module_type() {
        assert_eq!(
            resolve_type_ref("#/types/aws:s3/BucketRule:BucketRule", None),
            "crate::types::s3::BucketRule"
        );
    }

    #[test]
    fn resolve_ref_unrecognized_format() {
        assert_eq!(
            resolve_type_ref("some-other://ref", None),
            "serde_json::Value"
        );
    }

    // ---- Type resolution: primitives via resolve_raw_type ----

    #[test]
    fn resolve_primitive_string() {
        assert_eq!(
            resolve_raw_type(Some("string"), None, None, None, None, None),
            "String"
        );
    }

    #[test]
    fn resolve_primitive_integer() {
        assert_eq!(
            resolve_raw_type(Some("integer"), None, None, None, None, None),
            "i64"
        );
    }

    #[test]
    fn resolve_primitive_number() {
        assert_eq!(
            resolve_raw_type(Some("number"), None, None, None, None, None),
            "f64"
        );
    }

    #[test]
    fn resolve_primitive_boolean() {
        assert_eq!(
            resolve_raw_type(Some("boolean"), None, None, None, None, None),
            "bool"
        );
    }

    // ---- Type resolution: composites ----

    #[test]
    fn resolve_array_of_strings() {
        let items = TypeSpec {
            type_: Some("string".to_string()),
            ref_: None,
            items: None,
            additional_properties: None,
            one_of: None,
            plain: false,
        };
        assert_eq!(
            resolve_raw_type(Some("array"), None, Some(&items), None, None, None),
            "Vec<String>"
        );
    }

    #[test]
    fn resolve_array_no_items() {
        assert_eq!(
            resolve_raw_type(Some("array"), None, None, None, None, None),
            "Vec<serde_json::Value>"
        );
    }

    #[test]
    fn resolve_map_of_strings() {
        let ap = TypeSpec {
            type_: Some("string".to_string()),
            ref_: None,
            items: None,
            additional_properties: None,
            one_of: None,
            plain: false,
        };
        assert_eq!(
            resolve_raw_type(Some("object"), None, None, Some(&ap), None, None),
            "std::collections::HashMap<String, String>"
        );
    }

    #[test]
    fn resolve_bare_object() {
        assert_eq!(
            resolve_raw_type(Some("object"), None, None, None, None, None),
            "std::collections::HashMap<String, serde_json::Value>"
        );
    }

    #[test]
    fn resolve_one_of() {
        let variants = vec![
            TypeSpec {
                type_: Some("string".to_string()),
                ref_: None,
                items: None,
                additional_properties: None,
                one_of: None,
                plain: false,
            },
            TypeSpec {
                type_: Some("integer".to_string()),
                ref_: None,
                items: None,
                additional_properties: None,
                one_of: None,
                plain: false,
            },
        ];
        assert_eq!(
            resolve_raw_type(None, None, None, None, Some(&variants), None),
            "serde_json::Value"
        );
    }

    #[test]
    fn resolve_nested_array_of_refs() {
        let items = TypeSpec {
            type_: None,
            ref_: Some("#/types/aws:s3/Rule:Rule".to_string()),
            items: None,
            additional_properties: None,
            one_of: None,
            plain: false,
        };
        assert_eq!(
            resolve_raw_type(Some("array"), None, Some(&items), None, None, None),
            "Vec<crate::types::s3::Rule>"
        );
    }

    #[test]
    fn resolve_map_of_refs() {
        let ap = TypeSpec {
            type_: None,
            ref_: Some("#/types/aws:ec2/Tag:Tag".to_string()),
            items: None,
            additional_properties: None,
            one_of: None,
            plain: false,
        };
        assert_eq!(
            resolve_raw_type(Some("object"), None, None, Some(&ap), None, None),
            "std::collections::HashMap<String, crate::types::ec2::Tag>"
        );
    }

    #[test]
    fn resolve_ref_takes_precedence_over_type() {
        // When both $ref and type are present, $ref wins.
        assert_eq!(
            resolve_raw_type(
                Some("string"),
                Some("#/types/aws:s3/Bucket:Bucket"),
                None,
                None,
                None,
                None
            ),
            "crate::types::s3::Bucket"
        );
    }

    #[test]
    fn resolve_one_of_takes_precedence() {
        let variants = vec![TypeSpec {
            type_: Some("string".to_string()),
            ref_: None,
            items: None,
            additional_properties: None,
            one_of: None,
            plain: false,
        }];
        // oneOf takes precedence even if $ref is also present.
        assert_eq!(
            resolve_raw_type(
                None,
                Some("#/types/aws:s3/Bucket:Bucket"),
                None,
                None,
                Some(&variants),
                None,
            ),
            "serde_json::Value"
        );
    }

    #[test]
    fn resolve_no_type_info() {
        assert_eq!(
            resolve_raw_type(None, None, None, None, None, None),
            "serde_json::Value"
        );
    }

    // ---- Type resolution via PropertySpec ----

    #[test]
    fn resolve_property_type_string() {
        let prop = PropertySpec {
            type_: Some("string".to_string()),
            ref_: None,
            items: None,
            additional_properties: None,
            one_of: None,
            description: None,
            deprecation_message: None,
            secret: false,
            default_value: None,
            plain: false,
            replace_on_changes: false,
            will_replace_on_changes: false,
        };
        assert_eq!(resolve_property_type(&prop, None), "String");
    }

    #[test]
    fn resolve_property_type_ref() {
        let prop = PropertySpec {
            type_: None,
            ref_: Some("#/types/aws:s3/BucketCors:BucketCors".to_string()),
            items: None,
            additional_properties: None,
            one_of: None,
            description: None,
            deprecation_message: None,
            secret: false,
            default_value: None,
            plain: false,
            replace_on_changes: false,
            will_replace_on_changes: false,
        };
        assert_eq!(
            resolve_property_type(&prop, None),
            "crate::types::s3::BucketCors"
        );
    }

    // ---- TypeSpec resolution ----

    #[test]
    fn resolve_type_spec_integer() {
        let spec = TypeSpec {
            type_: Some("integer".to_string()),
            ref_: None,
            items: None,
            additional_properties: None,
            one_of: None,
            plain: false,
        };
        assert_eq!(resolve_type_spec(&spec, None), "i64");
    }

    // ---- Field resolution ----

    fn make_prop(type_name: &str) -> PropertySpec {
        PropertySpec {
            type_: Some(type_name.to_string()),
            ref_: None,
            items: None,
            additional_properties: None,
            one_of: None,
            description: None,
            deprecation_message: None,
            secret: false,
            default_value: None,
            plain: false,
            replace_on_changes: false,
            will_replace_on_changes: false,
        }
    }

    #[test]
    fn field_required_string() {
        let prop = make_prop("string");
        let field = resolve_field("bucketPrefix", &prop, true, None);
        assert_eq!(field.original_name, "bucketPrefix");
        assert_eq!(field.rust_name, "bucket_prefix");
        assert_eq!(field.rust_type, "String");
        assert!(field.required);
    }

    #[test]
    fn field_optional_integer() {
        let prop = make_prop("integer");
        let field = resolve_field("count", &prop, false, None);
        assert_eq!(field.rust_name, "count");
        assert_eq!(field.rust_type, "Option<i64>");
        assert!(!field.required);
    }

    #[test]
    fn field_keyword_escape() {
        let prop = make_prop("string");
        let field = resolve_field("type", &prop, true, None);
        assert_eq!(field.original_name, "type");
        assert_eq!(field.rust_name, "r#type");
        assert_eq!(field.rust_type, "String");
    }

    #[test]
    fn field_self_keyword_escape() {
        let prop = make_prop("boolean");
        let field = resolve_field("self", &prop, false, None);
        assert_eq!(field.original_name, "self");
        assert_eq!(field.rust_name, "self_");
        assert_eq!(field.rust_type, "Option<bool>");
    }

    #[test]
    fn field_preserves_metadata() {
        let prop = PropertySpec {
            type_: Some("string".to_string()),
            ref_: None,
            items: None,
            additional_properties: None,
            one_of: None,
            description: Some("A description".to_string()),
            deprecation_message: Some("Use other field".to_string()),
            secret: true,
            default_value: None,
            plain: false,
            replace_on_changes: false,
            will_replace_on_changes: false,
        };
        let field = resolve_field("myField", &prop, true, None);
        assert_eq!(field.description.as_deref(), Some("A description"));
        assert_eq!(field.deprecation.as_deref(), Some("Use other field"));
        assert!(field.secret);
    }

    #[test]
    fn field_optional_ref_type() {
        let prop = PropertySpec {
            type_: None,
            ref_: Some("#/types/aws:s3/BucketCors:BucketCors".to_string()),
            items: None,
            additional_properties: None,
            one_of: None,
            description: None,
            deprecation_message: None,
            secret: false,
            default_value: None,
            plain: false,
            replace_on_changes: false,
            will_replace_on_changes: false,
        };
        let field = resolve_field("corsRules", &prop, false, None);
        assert_eq!(field.rust_name, "cors_rules");
        assert_eq!(field.rust_type, "Option<crate::types::s3::BucketCors>");
    }

    #[test]
    fn resolve_fields_mixed_required() {
        let mut properties = BTreeMap::new();
        properties.insert("name".to_string(), make_prop("string"));
        properties.insert("count".to_string(), make_prop("integer"));
        properties.insert("enabled".to_string(), make_prop("boolean"));

        let required = vec!["name".to_string()];
        let fields = resolve_fields(&properties, &required, None);

        assert_eq!(fields.len(), 3);
        // BTreeMap iterates in alphabetical order: count, enabled, name
        assert_eq!(fields[0].rust_name, "count");
        assert_eq!(fields[0].rust_type, "Option<i64>");
        assert!(!fields[0].required);

        assert_eq!(fields[1].rust_name, "enabled");
        assert_eq!(fields[1].rust_type, "Option<bool>");
        assert!(!fields[1].required);

        assert_eq!(fields[2].rust_name, "name");
        assert_eq!(fields[2].rust_type, "String");
        assert!(fields[2].required);
    }
}
