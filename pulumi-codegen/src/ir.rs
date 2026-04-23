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

/// A resolved complex type — either an object, an enum, or a union.
#[derive(Debug)]
pub enum ResolvedType {
    Object(ResolvedObject),
    Enum(ResolvedEnum),
    Union(ResolvedUnion),
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

/// A resolved union (oneOf) type, emitted as a `#[serde(untagged)]` enum.
#[derive(Debug)]
pub struct ResolvedUnion {
    pub rust_name: String,
    pub file_name: String,
    pub variants: Vec<UnionVariant>,
}

/// A single variant of a resolved union.
#[derive(Debug)]
pub struct UnionVariant {
    pub rust_name: String,
    pub rust_type: String,
}

// ---------------------------------------------------------------------------
// Type resolution
// ---------------------------------------------------------------------------

/// Resolve a `$ref` string to a Rust type path.
///
/// Handles Pulumi built-in refs (`pulumi.json#/Any`, etc.) and local type
/// refs (`#/types/pkg:mod:Name`).
fn resolve_type_ref(ref_str: &str, module_format: Option<&str>) -> String {
    match ref_str {
        "pulumi.json#/Asset" => return "pulumi::Asset".to_string(),
        "pulumi.json#/Archive" => return "pulumi::Archive".to_string(),
        _ => {}
    }
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

/// Map a resolved Rust type string to a PascalCase variant name for use in a union enum.
fn type_to_variant_name(rust_type: &str) -> String {
    match rust_type {
        "String" => "String".to_string(),
        "i64" => "Integer".to_string(),
        "f64" => "Number".to_string(),
        "bool" => "Boolean".to_string(),
        "serde_json::Value" => "Any".to_string(),
        "pulumi::Asset" => "Asset".to_string(),
        "pulumi::Archive" => "Archive".to_string(),
        _ => {
            if let Some(inner) = rust_type.strip_prefix("Vec<").and_then(|s| s.strip_suffix('>')) {
                return format!("{}Array", type_to_variant_name(inner));
            }
            if let Some(inner) = rust_type
                .strip_prefix("std::collections::HashMap<String, ")
                .and_then(|s| s.strip_suffix('>'))
            {
                return format!("{}Map", type_to_variant_name(inner));
            }
            // For crate::types::... paths, take the last path segment.
            rust_type.split("::").last().unwrap_or(rust_type).to_string()
        }
    }
}

/// Resolve a `oneOf` variant list into a named union type, registering it in `unions`.
/// Returns the fully-qualified Rust type path (`crate::types::SomeOrOther`).
fn resolve_one_of(
    variants: &[TypeSpec],
    module_format: Option<&str>,
    unions: &mut BTreeMap<String, ResolvedUnion>,
) -> String {
    let union_variants: Vec<UnionVariant> = variants
        .iter()
        .map(|v| {
            let rust_type = resolve_type_spec(v, module_format, unions);
            let rust_name = type_to_variant_name(&rust_type);
            UnionVariant { rust_name, rust_type }
        })
        .collect();

    let rust_name: String = union_variants
        .iter()
        .map(|v| v.rust_name.as_str())
        .collect::<Vec<_>>()
        .join("Or");
    let file_name = camel_to_snake_case(&rust_name);

    unions.entry(rust_name.clone()).or_insert_with(|| ResolvedUnion {
        rust_name: rust_name.clone(),
        file_name,
        variants: union_variants,
    });

    format!("crate::types::{rust_name}")
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
    unions: &mut BTreeMap<String, ResolvedUnion>,
) -> String {
    // Union types → generate a named #[serde(untagged)] enum.
    if let Some(variants) = one_of {
        return resolve_one_of(variants, module_format, unions);
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
                .map(|i| resolve_type_spec(i, module_format, unions))
                .unwrap_or_else(|| "serde_json::Value".to_string());
            format!("Vec<{inner}>")
        }
        Some("object") => {
            if let Some(ap) = additional_properties {
                let inner = resolve_type_spec(ap, module_format, unions);
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
fn resolve_type_spec(spec: &TypeSpec, module_format: Option<&str>, unions: &mut BTreeMap<String, ResolvedUnion>) -> String {
    resolve_raw_type(
        spec.type_.as_deref(),
        spec.ref_.as_deref(),
        spec.items.as_deref(),
        spec.additional_properties.as_deref(),
        spec.one_of.as_deref(),
        module_format,
        unions,
    )
}

/// Resolve a [`PropertySpec`] to its base Rust type string (before optional wrapping).
fn resolve_property_type(prop: &PropertySpec, module_format: Option<&str>, unions: &mut BTreeMap<String, ResolvedUnion>) -> String {
    resolve_raw_type(
        prop.type_.as_deref(),
        prop.ref_.as_deref(),
        prop.items.as_deref(),
        prop.additional_properties.as_deref(),
        prop.one_of.as_deref(),
        module_format,
        unions,
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
    unions: &mut BTreeMap<String, ResolvedUnion>,
) -> ResolvedField {
    let snake = camel_to_snake_case(name);
    let rust_name = escape_rust_keyword(&snake);
    let base_type = resolve_property_type(prop, module_format, unions);
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
    unions: &mut BTreeMap<String, ResolvedUnion>,
) -> Vec<ResolvedField> {
    properties
        .iter()
        .map(|(name, prop)| {
            let is_required = required.contains(name);
            resolve_field(name, prop, is_required, module_format, unions)
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
    unions: &mut BTreeMap<String, ResolvedUnion>,
) -> Option<ResolvedResource> {
    if spec.is_overlay {
        return None;
    }
    let parsed = parse_type_token(token, module_format)?;
    let rust_name = to_pascal_case(&parsed.name);
    let file_name = type_name_to_file_name(&parsed.name);
    let input_fields = resolve_fields(&spec.input_properties, &spec.required_inputs, module_format, unions);
    let output_fields = resolve_fields(&spec.properties, &spec.required, module_format, unions);

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
    unions: &mut BTreeMap<String, ResolvedUnion>,
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
        .map(|obj| resolve_fields(&obj.properties, &obj.required, module_format, unions))
        .unwrap_or_default();

    let result_fields = spec
        .outputs
        .as_ref()
        .map(|obj| resolve_fields(&obj.properties, &obj.required, module_format, unions))
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
    unions: &mut BTreeMap<String, ResolvedUnion>,
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
        let fields = resolve_fields(&spec.properties, &spec.required, module_format, unions);
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
    let mut unions: BTreeMap<String, ResolvedUnion> = BTreeMap::new();

    // Resolve complex types.
    let mut types: BTreeMap<String, ResolvedType> = schema
        .types
        .iter()
        .filter_map(|(token, spec)| {
            resolve_complex_type(token, spec, module_format, &mut unions).map(|t| (token.clone(), t))
        })
        .collect();

    // Resolve resources, grouped by module.
    let mut module_resources: BTreeMap<String, Vec<ResolvedResource>> = BTreeMap::new();
    for (token, spec) in &schema.resources {
        if let Some(resource) = resolve_resource(token, spec, module_format, &mut unions) {
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
        if let Some(function) = resolve_function(token, spec, module_format, &mut unions) {
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

    // Merge generated union types into the types map. Use a synthetic key with
    // a `_union:` prefix to avoid collisions with real provider type tokens.
    for (rust_name, union) in unions {
        types.insert(format!("_union:{rust_name}"), ResolvedType::Union(union));
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
        assert_eq!(resolve_type_ref("pulumi.json#/Any", None), "serde_json::Value");
    }

    #[test]
    fn resolve_builtin_asset() {
        assert_eq!(resolve_type_ref("pulumi.json#/Asset", None), "pulumi::Asset");
    }

    #[test]
    fn resolve_builtin_archive() {
        assert_eq!(resolve_type_ref("pulumi.json#/Archive", None), "pulumi::Archive");
    }

    #[test]
    fn resolve_builtin_json() {
        assert_eq!(resolve_type_ref("pulumi.json#/Json", None), "serde_json::Value");
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
        assert_eq!(resolve_type_ref("some-other://ref", None), "serde_json::Value");
    }

    // ---- type_to_variant_name ----

    #[test]
    fn variant_name_primitives() {
        assert_eq!(type_to_variant_name("String"), "String");
        assert_eq!(type_to_variant_name("i64"), "Integer");
        assert_eq!(type_to_variant_name("f64"), "Number");
        assert_eq!(type_to_variant_name("bool"), "Boolean");
        assert_eq!(type_to_variant_name("serde_json::Value"), "Any");
        assert_eq!(type_to_variant_name("pulumi::Asset"), "Asset");
        assert_eq!(type_to_variant_name("pulumi::Archive"), "Archive");
    }

    #[test]
    fn variant_name_composites() {
        assert_eq!(type_to_variant_name("Vec<String>"), "StringArray");
        assert_eq!(type_to_variant_name("Vec<i64>"), "IntegerArray");
        assert_eq!(
            type_to_variant_name("std::collections::HashMap<String, String>"),
            "StringMap"
        );
        assert_eq!(
            type_to_variant_name("std::collections::HashMap<String, i64>"),
            "IntegerMap"
        );
    }

    #[test]
    fn variant_name_ref_type() {
        assert_eq!(type_to_variant_name("crate::types::s3::BucketObject"), "BucketObject");
        assert_eq!(type_to_variant_name("crate::types::Rule"), "Rule");
    }

    // ---- resolve_one_of: registers union and returns type path ----

    #[test]
    fn resolve_one_of() {
        let variants = vec![
            TypeSpec { type_: Some("string".to_string()), ref_: None, items: None, additional_properties: None, one_of: None, plain: false },
            TypeSpec { type_: Some("integer".to_string()), ref_: None, items: None, additional_properties: None, one_of: None, plain: false },
        ];
        let mut unions = BTreeMap::new();
        let result = resolve_raw_type(None, None, None, None, Some(&variants), None, &mut unions);
        assert_eq!(result, "crate::types::StringOrInteger");
        assert!(unions.contains_key("StringOrInteger"));
        let u = &unions["StringOrInteger"];
        assert_eq!(u.variants.len(), 2);
        assert_eq!(u.variants[0].rust_name, "String");
        assert_eq!(u.variants[0].rust_type, "String");
        assert_eq!(u.variants[1].rust_name, "Integer");
        assert_eq!(u.variants[1].rust_type, "i64");
    }

    #[test]
    fn resolve_one_of_deduplicates() {
        let variants = vec![
            TypeSpec { type_: Some("string".to_string()), ref_: None, items: None, additional_properties: None, one_of: None, plain: false },
            TypeSpec { type_: Some("boolean".to_string()), ref_: None, items: None, additional_properties: None, one_of: None, plain: false },
        ];
        let mut unions = BTreeMap::new();
        let r1 = resolve_raw_type(None, None, None, None, Some(&variants), None, &mut unions);
        let r2 = resolve_raw_type(None, None, None, None, Some(&variants), None, &mut unions);
        assert_eq!(r1, r2);
        assert_eq!(unions.len(), 1);
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
        let mut unions = BTreeMap::new();
        // oneOf takes precedence even if $ref is also present.
        let result = resolve_raw_type(None, Some("#/types/aws:s3/Bucket:Bucket"), None, None, Some(&variants), None, &mut unions);
        assert_eq!(result, "crate::types::String");
        assert!(unions.contains_key("String"));
    }

    // ---- Type resolution: primitives via resolve_raw_type ----

    #[test]
    fn resolve_primitive_string() {
        assert_eq!(
            resolve_raw_type(Some("string"), None, None, None, None, None, &mut BTreeMap::new()),
            "String"
        );
    }

    #[test]
    fn resolve_primitive_integer() {
        assert_eq!(
            resolve_raw_type(Some("integer"), None, None, None, None, None, &mut BTreeMap::new()),
            "i64"
        );
    }

    #[test]
    fn resolve_primitive_number() {
        assert_eq!(
            resolve_raw_type(Some("number"), None, None, None, None, None, &mut BTreeMap::new()),
            "f64"
        );
    }

    #[test]
    fn resolve_primitive_boolean() {
        assert_eq!(
            resolve_raw_type(Some("boolean"), None, None, None, None, None, &mut BTreeMap::new()),
            "bool"
        );
    }

    // ---- Type resolution: composites ----

    #[test]
    fn resolve_array_of_strings() {
        let items = TypeSpec { type_: Some("string".to_string()), ref_: None, items: None, additional_properties: None, one_of: None, plain: false };
        assert_eq!(
            resolve_raw_type(Some("array"), None, Some(&items), None, None, None, &mut BTreeMap::new()),
            "Vec<String>"
        );
    }

    #[test]
    fn resolve_array_no_items() {
        assert_eq!(
            resolve_raw_type(Some("array"), None, None, None, None, None, &mut BTreeMap::new()),
            "Vec<serde_json::Value>"
        );
    }

    #[test]
    fn resolve_map_of_strings() {
        let ap = TypeSpec { type_: Some("string".to_string()), ref_: None, items: None, additional_properties: None, one_of: None, plain: false };
        assert_eq!(
            resolve_raw_type(Some("object"), None, None, Some(&ap), None, None, &mut BTreeMap::new()),
            "std::collections::HashMap<String, String>"
        );
    }

    #[test]
    fn resolve_bare_object() {
        assert_eq!(
            resolve_raw_type(Some("object"), None, None, None, None, None, &mut BTreeMap::new()),
            "std::collections::HashMap<String, serde_json::Value>"
        );
    }

    #[test]
    fn resolve_nested_array_of_refs() {
        let items = TypeSpec { type_: None, ref_: Some("#/types/aws:s3/Rule:Rule".to_string()), items: None, additional_properties: None, one_of: None, plain: false };
        assert_eq!(
            resolve_raw_type(Some("array"), None, Some(&items), None, None, None, &mut BTreeMap::new()),
            "Vec<crate::types::s3::Rule>"
        );
    }

    #[test]
    fn resolve_map_of_refs() {
        let ap = TypeSpec { type_: None, ref_: Some("#/types/aws:ec2/Tag:Tag".to_string()), items: None, additional_properties: None, one_of: None, plain: false };
        assert_eq!(
            resolve_raw_type(Some("object"), None, None, Some(&ap), None, None, &mut BTreeMap::new()),
            "std::collections::HashMap<String, crate::types::ec2::Tag>"
        );
    }

    #[test]
    fn resolve_ref_takes_precedence_over_type() {
        assert_eq!(
            resolve_raw_type(Some("string"), Some("#/types/aws:s3/Bucket:Bucket"), None, None, None, None, &mut BTreeMap::new()),
            "crate::types::s3::Bucket"
        );
    }

    #[test]
    fn resolve_no_type_info() {
        assert_eq!(
            resolve_raw_type(None, None, None, None, None, None, &mut BTreeMap::new()),
            "serde_json::Value"
        );
    }

    // ---- Type resolution via PropertySpec ----

    #[test]
    fn resolve_property_type_string() {
        let prop = PropertySpec {
            type_: Some("string".to_string()),
            ref_: None, items: None, additional_properties: None, one_of: None,
            description: None, deprecation_message: None, secret: false,
            default_value: None, plain: false, replace_on_changes: false, will_replace_on_changes: false,
        };
        assert_eq!(resolve_property_type(&prop, None, &mut BTreeMap::new()), "String");
    }

    #[test]
    fn resolve_property_type_ref() {
        let prop = PropertySpec {
            type_: None,
            ref_: Some("#/types/aws:s3/BucketCors:BucketCors".to_string()),
            items: None, additional_properties: None, one_of: None,
            description: None, deprecation_message: None, secret: false,
            default_value: None, plain: false, replace_on_changes: false, will_replace_on_changes: false,
        };
        assert_eq!(resolve_property_type(&prop, None, &mut BTreeMap::new()), "crate::types::s3::BucketCors");
    }

    // ---- TypeSpec resolution ----

    #[test]
    fn resolve_type_spec_integer() {
        let spec = TypeSpec { type_: Some("integer".to_string()), ref_: None, items: None, additional_properties: None, one_of: None, plain: false };
        assert_eq!(resolve_type_spec(&spec, None, &mut BTreeMap::new()), "i64");
    }

    // ---- Field resolution ----

    fn make_prop(type_name: &str) -> PropertySpec {
        PropertySpec {
            type_: Some(type_name.to_string()),
            ref_: None, items: None, additional_properties: None, one_of: None,
            description: None, deprecation_message: None, secret: false,
            default_value: None, plain: false, replace_on_changes: false, will_replace_on_changes: false,
        }
    }

    #[test]
    fn field_required_string() {
        let prop = make_prop("string");
        let field = resolve_field("bucketPrefix", &prop, true, None, &mut BTreeMap::new());
        assert_eq!(field.original_name, "bucketPrefix");
        assert_eq!(field.rust_name, "bucket_prefix");
        assert_eq!(field.rust_type, "String");
        assert!(field.required);
    }

    #[test]
    fn field_optional_integer() {
        let prop = make_prop("integer");
        let field = resolve_field("count", &prop, false, None, &mut BTreeMap::new());
        assert_eq!(field.rust_name, "count");
        assert_eq!(field.rust_type, "Option<i64>");
        assert!(!field.required);
    }

    #[test]
    fn field_keyword_escape() {
        let prop = make_prop("string");
        let field = resolve_field("type", &prop, true, None, &mut BTreeMap::new());
        assert_eq!(field.original_name, "type");
        assert_eq!(field.rust_name, "r#type");
        assert_eq!(field.rust_type, "String");
    }

    #[test]
    fn field_self_keyword_escape() {
        let prop = make_prop("boolean");
        let field = resolve_field("self", &prop, false, None, &mut BTreeMap::new());
        assert_eq!(field.original_name, "self");
        assert_eq!(field.rust_name, "self_");
        assert_eq!(field.rust_type, "Option<bool>");
    }

    #[test]
    fn field_preserves_metadata() {
        let prop = PropertySpec {
            type_: Some("string".to_string()),
            ref_: None, items: None, additional_properties: None, one_of: None,
            description: Some("A description".to_string()),
            deprecation_message: Some("Use other field".to_string()),
            secret: true,
            default_value: None, plain: false, replace_on_changes: false, will_replace_on_changes: false,
        };
        let field = resolve_field("myField", &prop, true, None, &mut BTreeMap::new());
        assert_eq!(field.description.as_deref(), Some("A description"));
        assert_eq!(field.deprecation.as_deref(), Some("Use other field"));
        assert!(field.secret);
    }

    #[test]
    fn field_optional_ref_type() {
        let prop = PropertySpec {
            type_: None,
            ref_: Some("#/types/aws:s3/BucketCors:BucketCors".to_string()),
            items: None, additional_properties: None, one_of: None,
            description: None, deprecation_message: None, secret: false,
            default_value: None, plain: false, replace_on_changes: false, will_replace_on_changes: false,
        };
        let field = resolve_field("corsRules", &prop, false, None, &mut BTreeMap::new());
        assert_eq!(field.rust_name, "cors_rules");
        assert_eq!(field.rust_type, "Option<crate::types::s3::BucketCors>");
    }

    #[test]
    fn field_one_of_generates_union() {
        let prop = PropertySpec {
            type_: None, ref_: None,
            items: None, additional_properties: None,
            one_of: Some(vec![
                TypeSpec { type_: Some("string".to_string()), ref_: None, items: None, additional_properties: None, one_of: None, plain: false },
                TypeSpec { type_: Some("integer".to_string()), ref_: None, items: None, additional_properties: None, one_of: None, plain: false },
            ]),
            description: None, deprecation_message: None, secret: false,
            default_value: None, plain: false, replace_on_changes: false, will_replace_on_changes: false,
        };
        let mut unions = BTreeMap::new();
        let field = resolve_field("value", &prop, true, None, &mut unions);
        assert_eq!(field.rust_type, "crate::types::StringOrInteger");
        assert!(unions.contains_key("StringOrInteger"));
    }

    #[test]
    fn resolve_fields_mixed_required() {
        let mut properties = BTreeMap::new();
        properties.insert("name".to_string(), make_prop("string"));
        properties.insert("count".to_string(), make_prop("integer"));
        properties.insert("enabled".to_string(), make_prop("boolean"));

        let required = vec!["name".to_string()];
        let fields = resolve_fields(&properties, &required, None, &mut BTreeMap::new());

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

    // ---- Resource resolution ----

    #[test]
    fn resource_basic() {
        let mut input_properties = BTreeMap::new();
        input_properties.insert("name".to_string(), make_prop("string"));
        input_properties.insert("count".to_string(), make_prop("integer"));
        let mut properties = BTreeMap::new();
        properties.insert("id".to_string(), make_prop("string"));
        properties.insert("name".to_string(), make_prop("string"));

        let spec = ResourceSpec {
            description: Some("A test resource".to_string()),
            input_properties, properties,
            required_inputs: vec!["name".to_string()],
            required: vec!["id".to_string(), "name".to_string()],
            deprecation_message: None, is_component: false, is_overlay: false,
            methods: BTreeMap::new(), state_inputs: None, aliases: vec![],
        };

        let res = resolve_resource("test:index/myResource:MyResource", &spec, None, &mut BTreeMap::new()).unwrap();
        assert_eq!(res.type_token, "test:index/myResource:MyResource");
        assert_eq!(res.rust_name, "MyResource");
        assert_eq!(res.file_name, "my_resource");
        assert_eq!(res.description.as_deref(), Some("A test resource"));
        assert!(!res.is_component);
        assert_eq!(res.input_fields.len(), 2);
        let count_field = res.input_fields.iter().find(|f| f.rust_name == "count").unwrap();
        assert_eq!(count_field.rust_type, "Option<i64>");
        let name_input = res.input_fields.iter().find(|f| f.rust_name == "name").unwrap();
        assert_eq!(name_input.rust_type, "String");
        assert!(name_input.required);
        assert_eq!(res.output_fields.len(), 2);
        let id_field = res.output_fields.iter().find(|f| f.rust_name == "id").unwrap();
        assert_eq!(id_field.rust_type, "String");
        assert!(id_field.required);
    }

    #[test]
    fn resource_overlay_skipped() {
        let spec = ResourceSpec {
            description: None, input_properties: BTreeMap::new(), properties: BTreeMap::new(),
            required_inputs: vec![], required: vec![], deprecation_message: None,
            is_component: false, is_overlay: true, methods: BTreeMap::new(), state_inputs: None, aliases: vec![],
        };
        assert!(resolve_resource("test:index/overlay:Overlay", &spec, None, &mut BTreeMap::new()).is_none());
    }

    #[test]
    fn resource_component() {
        let spec = ResourceSpec {
            description: None, input_properties: BTreeMap::new(), properties: BTreeMap::new(),
            required_inputs: vec![], required: vec![], deprecation_message: None,
            is_component: true, is_overlay: false, methods: BTreeMap::new(), state_inputs: None, aliases: vec![],
        };
        let res = resolve_resource("test:index/comp:MyComponent", &spec, None, &mut BTreeMap::new()).unwrap();
        assert!(res.is_component);
        assert_eq!(res.rust_name, "MyComponent");
    }

    #[test]
    fn resource_deprecated() {
        let spec = ResourceSpec {
            description: None, input_properties: BTreeMap::new(), properties: BTreeMap::new(),
            required_inputs: vec![], required: vec![],
            deprecation_message: Some("Use NewResource instead".to_string()),
            is_component: false, is_overlay: false, methods: BTreeMap::new(), state_inputs: None, aliases: vec![],
        };
        let res = resolve_resource("test:index/old:OldResource", &spec, None, &mut BTreeMap::new()).unwrap();
        assert_eq!(res.deprecation.as_deref(), Some("Use NewResource instead"));
    }

    // ---- Function resolution ----

    #[test]
    fn function_basic() {
        let spec = FunctionSpec {
            description: Some("Get a widget".to_string()),
            inputs: Some(crate::schema::ObjectTypeSpec {
                properties: { let mut m = BTreeMap::new(); m.insert("id".to_string(), make_prop("string")); m },
                required: vec!["id".to_string()],
                description: None, type_: None,
            }),
            outputs: Some(crate::schema::ObjectTypeSpec {
                properties: {
                    let mut m = BTreeMap::new();
                    m.insert("name".to_string(), make_prop("string"));
                    m.insert("value".to_string(), make_prop("number"));
                    m
                },
                required: vec!["name".to_string(), "value".to_string()],
                description: None, type_: None,
            }),
            deprecation_message: None, is_overlay: false, multi_argument_inputs: None,
        };

        let func = resolve_function("test:index/getWidget:getWidget", &spec, None, &mut BTreeMap::new()).unwrap();
        assert_eq!(func.rust_name, "GetWidget");
        assert_eq!(func.file_name, "get_widget");
        assert_eq!(func.description.as_deref(), Some("Get a widget"));
        assert_eq!(func.arg_fields.len(), 1);
        assert_eq!(func.arg_fields[0].rust_name, "id");
        assert_eq!(func.arg_fields[0].rust_type, "String");
        assert_eq!(func.result_fields.len(), 2);
        let name_field = func.result_fields.iter().find(|f| f.rust_name == "name").unwrap();
        assert_eq!(name_field.rust_type, "String");
        let value_field = func.result_fields.iter().find(|f| f.rust_name == "value").unwrap();
        assert_eq!(value_field.rust_type, "f64");
    }

    #[test]
    fn function_no_inputs_or_outputs() {
        let spec = FunctionSpec { description: None, inputs: None, outputs: None, deprecation_message: None, is_overlay: false, multi_argument_inputs: None };
        let func = resolve_function("test:index/doThing:doThing", &spec, None, &mut BTreeMap::new()).unwrap();
        assert_eq!(func.rust_name, "DoThing");
        assert!(func.arg_fields.is_empty());
        assert!(func.result_fields.is_empty());
    }

    #[test]
    fn function_overlay_skipped() {
        let spec = FunctionSpec { description: None, inputs: None, outputs: None, deprecation_message: None, is_overlay: true, multi_argument_inputs: None };
        assert!(resolve_function("test:index/overlay:overlay", &spec, None, &mut BTreeMap::new()).is_none());
    }

    // ---- Complex type resolution ----

    #[test]
    fn complex_type_object() {
        let spec = ComplexTypeSpec {
            description: Some("A lifecycle rule".to_string()),
            type_: Some("object".to_string()),
            properties: {
                let mut m = BTreeMap::new();
                m.insert("enabled".to_string(), make_prop("boolean"));
                m.insert("prefix".to_string(), make_prop("string"));
                m
            },
            required: vec!["enabled".to_string()],
            enum_values: None, is_overlay: false,
        };

        let resolved = resolve_complex_type(
            "aws:s3/BucketLifecycleRule:BucketLifecycleRule", &spec, None, &mut BTreeMap::new()
        ).unwrap();
        match resolved {
            ResolvedType::Object(obj) => {
                assert_eq!(obj.rust_name, "BucketLifecycleRule");
                assert_eq!(obj.module, "s3");
                assert_eq!(obj.file_name, "bucket_lifecycle_rule");
                assert_eq!(obj.fields.len(), 2);
                let enabled = obj.fields.iter().find(|f| f.rust_name == "enabled").unwrap();
                assert_eq!(enabled.rust_type, "bool");
                assert!(enabled.required);
                let prefix = obj.fields.iter().find(|f| f.rust_name == "prefix").unwrap();
                assert_eq!(prefix.rust_type, "Option<String>");
            }
            other => panic!("Expected object, got {other:?}"),
        }
    }

    #[test]
    fn complex_type_enum_string() {
        let spec = ComplexTypeSpec {
            description: Some("Canned ACL".to_string()),
            type_: Some("string".to_string()),
            properties: BTreeMap::new(),
            required: vec![],
            enum_values: Some(vec![
                crate::schema::EnumValueSpec { name: Some("Private".to_string()), value: serde_json::Value::String("private".to_string()), description: Some("Private access".to_string()), deprecation_message: None },
                crate::schema::EnumValueSpec { name: Some("PublicRead".to_string()), value: serde_json::Value::String("public-read".to_string()), description: None, deprecation_message: None },
                crate::schema::EnumValueSpec { name: None, value: serde_json::Value::String("public-read-write".to_string()), description: None, deprecation_message: None },
            ]),
            is_overlay: false,
        };

        let resolved = resolve_complex_type("aws:s3/CannedAcl:CannedAcl", &spec, None, &mut BTreeMap::new()).unwrap();
        match resolved {
            ResolvedType::Enum(e) => {
                assert_eq!(e.rust_name, "CannedAcl");
                assert_eq!(e.module, "s3");
                assert_eq!(e.underlying_type, "String");
                assert_eq!(e.variants.len(), 3);
                assert_eq!(e.variants[0].rust_name, "Private");
                assert_eq!(e.variants[0].value, "private");
                assert_eq!(e.variants[0].description.as_deref(), Some("Private access"));
                assert_eq!(e.variants[1].rust_name, "PublicRead");
                assert_eq!(e.variants[1].value, "public-read");
                assert_eq!(e.variants[2].rust_name, "PublicReadWrite");
                assert_eq!(e.variants[2].value, "public-read-write");
            }
            other => panic!("Expected enum, got {other:?}"),
        }
    }

    #[test]
    fn complex_type_enum_integer() {
        let spec = ComplexTypeSpec {
            description: None,
            type_: Some("integer".to_string()),
            properties: BTreeMap::new(),
            required: vec![],
            enum_values: Some(vec![
                crate::schema::EnumValueSpec { name: Some("Small".to_string()), value: serde_json::json!(1), description: None, deprecation_message: None },
                crate::schema::EnumValueSpec { name: None, value: serde_json::json!(99), description: None, deprecation_message: None },
            ]),
            is_overlay: false,
        };

        let resolved = resolve_complex_type("test:index/Size:Size", &spec, None, &mut BTreeMap::new()).unwrap();
        match resolved {
            ResolvedType::Enum(e) => {
                assert_eq!(e.underlying_type, "i64");
                assert_eq!(e.module, "");
                assert_eq!(e.variants[0].rust_name, "Small");
                assert_eq!(e.variants[0].value, "1");
                assert_eq!(e.variants[1].rust_name, "V99");
                assert_eq!(e.variants[1].value, "99");
            }
            other => panic!("Expected enum, got {other:?}"),
        }
    }

    #[test]
    fn complex_type_overlay_skipped() {
        let spec = ComplexTypeSpec {
            description: None, type_: Some("object".to_string()), properties: BTreeMap::new(),
            required: vec![], enum_values: None, is_overlay: true,
        };
        assert!(resolve_complex_type("test:index/overlay:Overlay", &spec, None, &mut BTreeMap::new()).is_none());
    }

    // ---- resolve_package with union types ----

    #[test]
    fn resolve_package_with_one_of() {
        let json = r#"{
            "name": "test",
            "resources": {
                "test:index/thing:Thing": {
                    "inputProperties": {
                        "value": {
                            "oneOf": [
                                { "type": "string" },
                                { "type": "integer" }
                            ]
                        }
                    },
                    "properties": {
                        "value": {
                            "oneOf": [
                                { "type": "string" },
                                { "type": "integer" }
                            ]
                        }
                    },
                    "requiredInputs": ["value"],
                    "required": ["value"]
                }
            }
        }"#;
        let schema: crate::schema::PackageSchema = serde_json::from_str(json).unwrap();
        let pkg = resolve_package(&schema);

        // Union type should appear in package types.
        let union_key = "_union:StringOrInteger";
        assert!(pkg.types.contains_key(union_key), "expected union type in package types");
        match &pkg.types[union_key] {
            ResolvedType::Union(u) => {
                assert_eq!(u.rust_name, "StringOrInteger");
                assert_eq!(u.file_name, "string_or_integer");
                assert_eq!(u.variants.len(), 2);
                assert_eq!(u.variants[0].rust_name, "String");
                assert_eq!(u.variants[1].rust_name, "Integer");
            }
            other => panic!("Expected union, got {other:?}"),
        }

        // Field on the resource should reference the generated union type.
        let module = pkg.modules.get("").unwrap();
        let res = module.resources.iter().find(|r| r.rust_name == "Thing").unwrap();
        let val_input = res.input_fields.iter().find(|f| f.rust_name == "value").unwrap();
        assert_eq!(val_input.rust_type, "crate::types::StringOrInteger");
    }
}
