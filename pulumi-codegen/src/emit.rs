//! Code emission: generates Rust source files from a [`ResolvedPackage`].
//!
//! Walks the resolved IR and writes a complete Rust crate to disk, including
//! `Cargo.toml`, `src/lib.rs`, per-module `mod.rs`, and per-resource/function/type
//! `.rs` files.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

use crate::ir::{
    ResolvedEnum, ResolvedField, ResolvedFunction, ResolvedObject, ResolvedPackage,
    ResolvedResource, ResolvedType, ResolvedUnion,
};

/// Options for code emission.
pub struct EmitOptions {
    /// If set, use a path dependency for the `pulumi` crate instead of a
    /// version from crates.io. Useful for local development and validation.
    pub pulumi_crate_path: Option<String>,
}

impl Default for EmitOptions {
    fn default() -> Self {
        Self {
            pulumi_crate_path: None,
        }
    }
}

/// Generate a complete Rust crate from a [`ResolvedPackage`] and write it to `out_dir`.
///
/// Creates the directory structure:
/// ```text
/// {out_dir}/
/// ├── Cargo.toml
/// └── src/
///     ├── lib.rs
///     ├── {resource}.rs          (root module resources)
///     ├── {module}/
///     │   ├── mod.rs
///     │   └── {resource}.rs
///     └── types/
///         ├── mod.rs
///         ├── {type}.rs          (root module types)
///         └── {module}/
///             ├── mod.rs
///             └── {type}.rs
/// ```
pub fn emit_package(package: &ResolvedPackage, out_dir: &Path) -> std::io::Result<()> {
    emit_package_with_options(package, out_dir, &EmitOptions::default())
}

/// Generate a complete Rust crate with custom options.
pub fn emit_package_with_options(
    package: &ResolvedPackage,
    out_dir: &Path,
    options: &EmitOptions,
) -> std::io::Result<()> {
    let src_dir = out_dir.join("src");
    std::fs::create_dir_all(&src_dir)?;

    // Cargo.toml
    std::fs::write(out_dir.join("Cargo.toml"), emit_cargo_toml(package, options))?;

    // Collect all file entries for lib.rs generation.
    let root_module = package.modules.get("");
    let non_root_modules: BTreeMap<&str, _> = package
        .modules
        .iter()
        .filter(|(k, _)| !k.is_empty())
        .map(|(k, v)| (k.as_str(), v))
        .collect();

    // Group types by module for the types/ subdirectory.
    let mut types_by_module: BTreeMap<String, Vec<&ResolvedType>> = BTreeMap::new();
    for resolved_type in package.types.values() {
        let module = match resolved_type {
            ResolvedType::Object(o) => o.module.clone(),
            ResolvedType::Enum(e) => e.module.clone(),
            ResolvedType::Union(_) => String::new(),
        };
        types_by_module.entry(module).or_default().push(resolved_type);
    }

    // Emit root module resource/function files.
    if let Some(root) = root_module {
        for resource in &root.resources {
            let path = src_dir.join(format!("{}.rs", resource.file_name));
            std::fs::write(&path, emit_resource_file(resource, &package.version))?;
        }
        for function in &root.functions {
            let path = src_dir.join(format!("{}.rs", function.file_name));
            std::fs::write(&path, emit_function_file(function, &package.version))?;
        }
    }

    // Emit non-root module directories.
    for (module_name, module) in &non_root_modules {
        let mod_dir = src_dir.join(module_name);
        std::fs::create_dir_all(&mod_dir)?;

        for resource in &module.resources {
            let path = mod_dir.join(format!("{}.rs", resource.file_name));
            std::fs::write(&path, emit_resource_file(resource, &package.version))?;
        }
        for function in &module.functions {
            let path = mod_dir.join(format!("{}.rs", function.file_name));
            std::fs::write(&path, emit_function_file(function, &package.version))?;
        }

        // mod.rs for this module
        std::fs::write(mod_dir.join("mod.rs"), emit_module_mod(module))?;
    }

    // Emit types/ directory.
    if !types_by_module.is_empty() {
        let types_dir = src_dir.join("types");
        std::fs::create_dir_all(&types_dir)?;

        // Root-level types go directly in types/
        if let Some(root_types) = types_by_module.get("") {
            for resolved_type in root_types {
                let (file_name, content) = emit_type_file(resolved_type);
                std::fs::write(types_dir.join(format!("{file_name}.rs")), content)?;
            }
        }

        // Module-level types go in types/{module}/
        for (module_name, types) in &types_by_module {
            if module_name.is_empty() {
                continue;
            }
            let mod_dir = types_dir.join(module_name);
            std::fs::create_dir_all(&mod_dir)?;

            for resolved_type in types {
                let (file_name, content) = emit_type_file(resolved_type);
                std::fs::write(mod_dir.join(format!("{file_name}.rs")), content)?;
            }

            // mod.rs for this types submodule
            std::fs::write(
                mod_dir.join("mod.rs"),
                emit_types_submodule_mod(types),
            )?;
        }

        // types/mod.rs
        std::fs::write(
            types_dir.join("mod.rs"),
            emit_types_mod(&types_by_module),
        )?;
    }

    // src/lib.rs
    std::fs::write(
        src_dir.join("lib.rs"),
        emit_lib_rs(root_module, &non_root_modules, &types_by_module),
    )?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Cargo.toml
// ---------------------------------------------------------------------------

fn emit_cargo_toml(package: &ResolvedPackage, options: &EmitOptions) -> String {
    let (pulumi_dep, pulumi_core_dep) = match &options.pulumi_crate_path {
        Some(path) => {
            // Derive the pulumi-core path from the pulumi crate path
            // (assumes sibling directory layout: ../pulumi-core relative to ../pulumi).
            let pulumi_path = std::path::Path::new(path);
            let core_path = pulumi_path
                .parent()
                .map(|p| p.join("pulumi-core"))
                .unwrap_or_else(|| std::path::PathBuf::from("../pulumi-core"));
            (
                format!("pulumi = {{ path = \"{path}\", features = [\"macros\"] }}"),
                format!(
                    "pulumi-core = {{ path = \"{}\" }}",
                    core_path.display()
                ),
            )
        }
        None => (
            "pulumi = { version = \"0.1\", features = [\"macros\"] }".to_string(),
            "pulumi-core = { version = \"0.1\" }".to_string(),
        ),
    };
    format!(
        r#"[package]
name = "pulumi-{name}"
version = "{version}"
edition = "2024"
description = "Pulumi SDK for the {name} provider"
license = "Apache-2.0"

[lib]
doctest = false

[dependencies]
{pulumi_dep}
{pulumi_core_dep}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
"#,
        name = package.name,
        version = package.version,
    )
}

// ---------------------------------------------------------------------------
// src/lib.rs
// ---------------------------------------------------------------------------

fn emit_lib_rs(
    root_module: Option<&crate::ir::ResolvedModule>,
    non_root_modules: &BTreeMap<&str, &crate::ir::ResolvedModule>,
    types_by_module: &BTreeMap<String, Vec<&ResolvedType>>,
) -> String {
    let mut out = String::new();

    // Types module declaration (if any types exist).
    if !types_by_module.is_empty() {
        out.push_str("pub mod types;\n");
    }

    // Non-root module declarations.
    for module_name in non_root_modules.keys() {
        writeln!(out, "pub mod {module_name};").unwrap();
    }

    // Root module resource/function file declarations + re-exports.
    if let Some(root) = root_module {
        for resource in &root.resources {
            writeln!(out, "mod {};", resource.file_name).unwrap();
            writeln!(out, "pub use {}::*;", resource.file_name).unwrap();
        }
        for function in &root.functions {
            writeln!(out, "mod {};", function.file_name).unwrap();
            writeln!(out, "pub use {}::*;", function.file_name).unwrap();
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Module mod.rs (for non-root resource/function modules)
// ---------------------------------------------------------------------------

fn emit_module_mod(module: &crate::ir::ResolvedModule) -> String {
    let mut out = String::new();
    for resource in &module.resources {
        writeln!(out, "mod {};", resource.file_name).unwrap();
        writeln!(out, "pub use {}::*;", resource.file_name).unwrap();
    }
    for function in &module.functions {
        writeln!(out, "mod {};", function.file_name).unwrap();
        writeln!(out, "pub use {}::*;", function.file_name).unwrap();
    }
    out
}

// ---------------------------------------------------------------------------
// types/mod.rs
// ---------------------------------------------------------------------------

fn emit_types_mod(types_by_module: &BTreeMap<String, Vec<&ResolvedType>>) -> String {
    let mut out = String::new();

    // Root-level type file declarations + re-exports.
    if let Some(root_types) = types_by_module.get("") {
        for resolved_type in root_types {
            let file_name = type_file_name(resolved_type);
            writeln!(out, "mod {file_name};").unwrap();
            writeln!(out, "pub use {file_name}::*;").unwrap();
        }
    }

    // Submodule declarations.
    for module_name in types_by_module.keys() {
        if module_name.is_empty() {
            continue;
        }
        writeln!(out, "pub mod {module_name};").unwrap();
    }

    out
}

fn emit_types_submodule_mod(types: &[&ResolvedType]) -> String {
    let mut out = String::new();
    for resolved_type in types {
        let file_name = type_file_name(resolved_type);
        writeln!(out, "mod {file_name};").unwrap();
        writeln!(out, "pub use {file_name}::*;").unwrap();
    }
    out
}

fn type_file_name(resolved_type: &ResolvedType) -> &str {
    match resolved_type {
        ResolvedType::Object(o) => &o.file_name,
        ResolvedType::Enum(e) => &e.file_name,
        ResolvedType::Union(u) => &u.file_name,
    }
}

// ---------------------------------------------------------------------------
// Per-resource file
// ---------------------------------------------------------------------------

fn emit_resource_file(resource: &ResolvedResource, version: &str) -> String {
    let mut out = String::new();

    out.push_str("use serde::{Deserialize, Serialize};\n\n");

    // Args struct
    emit_doc_comment(&mut out, resource.description.as_deref());
    emit_deprecation_attr(&mut out, resource.deprecation.as_deref());
    writeln!(out, "#[derive(Serialize)]").unwrap();
    writeln!(out, "pub struct {}Args {{", resource.rust_name).unwrap();
    emit_fields(&mut out, &resource.input_fields, true);
    writeln!(out, "}}\n").unwrap();

    // Outputs struct
    emit_doc_comment(&mut out, resource.description.as_deref());
    emit_deprecation_attr(&mut out, resource.deprecation.as_deref());
    writeln!(out, "#[derive(Deserialize, Clone)]").unwrap();
    writeln!(out, "pub struct {}Outputs {{", resource.rust_name).unwrap();
    emit_fields(&mut out, &resource.output_fields, false);
    writeln!(out, "}}\n").unwrap();

    // Resource unit struct with derive macro
    let derive = if resource.is_component {
        "pulumi::ComponentResource"
    } else {
        "pulumi::Resource"
    };
    let token_attr = if resource.is_component {
        "type_token"
    } else {
        "type_token"
    };

    emit_doc_comment(&mut out, resource.description.as_deref());
    emit_deprecation_attr(&mut out, resource.deprecation.as_deref());
    writeln!(out, "#[derive({derive})]").unwrap();
    writeln!(
        out,
        "#[pulumi({token_attr} = \"{}\")]",
        resource.type_token
    )
    .unwrap();
    writeln!(out, "#[pulumi(inputs = {}Args)]", resource.rust_name).unwrap();
    writeln!(out, "#[pulumi(outputs = {}Outputs)]", resource.rust_name).unwrap();
    writeln!(out, "#[pulumi(version = \"{version}\")]").unwrap();
    writeln!(out, "pub struct {};", resource.rust_name).unwrap();

    out
}

// ---------------------------------------------------------------------------
// Per-function file
// ---------------------------------------------------------------------------

fn emit_function_file(function: &ResolvedFunction, version: &str) -> String {
    let mut out = String::new();

    out.push_str("use serde::{Deserialize, Serialize};\n\n");

    // Args struct
    emit_doc_comment(&mut out, function.description.as_deref());
    emit_deprecation_attr(&mut out, function.deprecation.as_deref());
    writeln!(out, "#[derive(Serialize)]").unwrap();
    writeln!(out, "pub struct {}Args {{", function.rust_name).unwrap();
    emit_fields(&mut out, &function.arg_fields, true);
    writeln!(out, "}}\n").unwrap();

    // Result struct
    writeln!(out, "#[derive(Deserialize)]").unwrap();
    writeln!(out, "pub struct {}Result {{", function.rust_name).unwrap();
    emit_fields(&mut out, &function.result_fields, false);
    writeln!(out, "}}\n").unwrap();

    // Function unit struct with derive macro
    emit_doc_comment(&mut out, function.description.as_deref());
    emit_deprecation_attr(&mut out, function.deprecation.as_deref());
    writeln!(out, "#[derive(pulumi::ProviderFunction)]").unwrap();
    writeln!(out, "#[pulumi(token = \"{}\")]", function.token).unwrap();
    writeln!(out, "#[pulumi(args = {}Args)]", function.rust_name).unwrap();
    writeln!(out, "#[pulumi(returns = {}Result)]", function.rust_name).unwrap();
    writeln!(out, "#[pulumi(version = \"{version}\")]").unwrap();
    writeln!(out, "pub struct {};", function.rust_name).unwrap();

    out
}

// ---------------------------------------------------------------------------
// Per-type file (object or enum)
// ---------------------------------------------------------------------------

fn emit_type_file(resolved_type: &ResolvedType) -> (&str, String) {
    match resolved_type {
        ResolvedType::Object(obj) => (&obj.file_name, emit_object_type(obj)),
        ResolvedType::Enum(e) => (&e.file_name, emit_enum_type(e)),
        ResolvedType::Union(u) => (&u.file_name, emit_union_type(u)),
    }
}

fn emit_object_type(obj: &ResolvedObject) -> String {
    let mut out = String::new();

    out.push_str("use serde::{Deserialize, Serialize};\n\n");

    emit_doc_comment(&mut out, obj.description.as_deref());
    writeln!(out, "#[derive(Serialize, Deserialize, Clone)]").unwrap();
    writeln!(out, "pub struct {} {{", obj.rust_name).unwrap();
    // Object type fields can appear in both inputs and outputs, so emit
    // skip_serializing_if on optional fields (same as input fields).
    emit_fields(&mut out, &obj.fields, true);
    writeln!(out, "}}").unwrap();

    out
}

fn emit_enum_type(e: &ResolvedEnum) -> String {
    let mut out = String::new();

    out.push_str("use serde::{Deserialize, Serialize};\n\n");

    emit_doc_comment(&mut out, e.description.as_deref());
    writeln!(out, "#[derive(Serialize, Deserialize, Clone, PartialEq)]").unwrap();
    writeln!(out, "pub enum {} {{", e.rust_name).unwrap();
    for variant in &e.variants {
        emit_doc_comment(&mut out, variant.description.as_deref());
        emit_deprecation_attr(&mut out, variant.deprecation.as_deref());
        writeln!(out, "    #[serde(rename = \"{}\")]", variant.value).unwrap();
        writeln!(out, "    {},", variant.rust_name).unwrap();
    }
    writeln!(out, "}}").unwrap();

    out
}

fn emit_union_type(u: &ResolvedUnion) -> String {
    let mut out = String::new();
    out.push_str("use serde::{Deserialize, Serialize};\n\n");
    writeln!(out, "#[derive(Serialize, Deserialize, Clone)]").unwrap();
    writeln!(out, "#[serde(untagged)]").unwrap();
    writeln!(out, "pub enum {} {{", u.rust_name).unwrap();
    for variant in &u.variants {
        writeln!(out, "    {}({}),", variant.rust_name, variant.rust_type).unwrap();
    }
    writeln!(out, "}}").unwrap();
    out
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn emit_fields(out: &mut String, fields: &[ResolvedField], is_input: bool) {
    for field in fields {
        emit_doc_comment(out, field.description.as_deref());
        emit_deprecation_attr(out, field.deprecation.as_deref());

        if is_input && !field.required {
            writeln!(
                out,
                "    #[serde(rename = \"{}\", skip_serializing_if = \"Option::is_none\")]",
                field.original_name
            )
            .unwrap();
        } else {
            writeln!(out, "    #[serde(rename = \"{}\")]", field.original_name).unwrap();
        }

        writeln!(out, "    pub {}: {},", field.rust_name, field.rust_type).unwrap();
    }
}

fn emit_doc_comment(out: &mut String, description: Option<&str>) {
    if let Some(desc) = description {
        for line in desc.lines() {
            writeln!(out, "    /// {line}").unwrap();
        }
    }
}

fn emit_deprecation_attr(out: &mut String, deprecation: Option<&str>) {
    if let Some(msg) = deprecation {
        // Escape quotes in the deprecation message.
        let escaped = msg.replace('"', "\\\"");
        writeln!(out, "    #[deprecated(note = \"{escaped}\")]").unwrap();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{
        ResolvedEnum, ResolvedEnumVariant, ResolvedField, ResolvedFunction, ResolvedModule,
        ResolvedObject, ResolvedPackage, ResolvedResource, ResolvedType, ResolvedUnion,
    };
    use std::collections::BTreeMap;

    fn make_field(name: &str, rust_name: &str, rust_type: &str, required: bool) -> ResolvedField {
        ResolvedField {
            original_name: name.to_string(),
            rust_name: rust_name.to_string(),
            rust_type: rust_type.to_string(),
            description: None,
            deprecation: None,
            secret: false,
            required,
        }
    }

    // ---- Cargo.toml ----

    #[test]
    fn cargo_toml_output() {
        let package = ResolvedPackage {
            name: "random".to_string(),
            version: "4.16.0".to_string(),
            modules: BTreeMap::new(),
            types: BTreeMap::new(),
        };
        let toml = emit_cargo_toml(&package, &EmitOptions::default());
        assert!(toml.contains("name = \"pulumi-random\""));
        assert!(toml.contains("version = \"4.16.0\""));
        assert!(toml.contains("pulumi = {"));
        assert!(toml.contains("serde = {"));
        assert!(toml.contains("serde_json = \"1\""));
    }

    // ---- Resource file ----

    #[test]
    fn resource_file_basic() {
        let resource = ResolvedResource {
            type_token: "test:index/thing:Thing".to_string(),
            rust_name: "Thing".to_string(),
            file_name: "thing".to_string(),
            description: Some("A thing resource.".to_string()),
            deprecation: None,
            is_component: false,
            input_fields: vec![
                make_field("name", "name", "String", true),
                make_field("count", "count", "Option<i64>", false),
            ],
            output_fields: vec![
                make_field("id", "id", "String", true),
                make_field("name", "name", "String", true),
            ],
        };
        let code = emit_resource_file(&resource, "1.0.0");

        // Args struct
        assert!(code.contains("#[derive(Serialize)]"));
        assert!(code.contains("pub struct ThingArgs {"));
        assert!(code.contains("#[serde(rename = \"name\")]"));
        assert!(code.contains("pub name: String,"));
        assert!(code.contains(
            "#[serde(rename = \"count\", skip_serializing_if = \"Option::is_none\")]"
        ));
        assert!(code.contains("pub count: Option<i64>,"));

        // Outputs struct
        assert!(code.contains("#[derive(Deserialize, Clone)]"));
        assert!(code.contains("pub struct ThingOutputs {"));

        // Resource derive
        assert!(code.contains("#[derive(pulumi::Resource)]"));
        assert!(code.contains("#[pulumi(type_token = \"test:index/thing:Thing\")]"));
        assert!(code.contains("#[pulumi(inputs = ThingArgs)]"));
        assert!(code.contains("#[pulumi(outputs = ThingOutputs)]"));
        assert!(code.contains("#[pulumi(version = \"1.0.0\")]"));
        assert!(code.contains("pub struct Thing;"));
    }

    #[test]
    fn resource_file_component() {
        let resource = ResolvedResource {
            type_token: "test:index/comp:MyComp".to_string(),
            rust_name: "MyComp".to_string(),
            file_name: "my_comp".to_string(),
            description: None,
            deprecation: None,
            is_component: true,
            input_fields: vec![],
            output_fields: vec![],
        };
        let code = emit_resource_file(&resource, "1.0.0");
        assert!(code.contains("#[derive(pulumi::ComponentResource)]"));
    }

    #[test]
    fn resource_file_deprecated() {
        let resource = ResolvedResource {
            type_token: "test:index/old:Old".to_string(),
            rust_name: "Old".to_string(),
            file_name: "old".to_string(),
            description: None,
            deprecation: Some("Use New instead".to_string()),
            is_component: false,
            input_fields: vec![],
            output_fields: vec![],
        };
        let code = emit_resource_file(&resource, "1.0.0");
        assert!(code.contains("#[deprecated(note = \"Use New instead\")]"));
    }

    // ---- Function file ----

    #[test]
    fn function_file_basic() {
        let function = ResolvedFunction {
            token: "test:index/getWidget:getWidget".to_string(),
            rust_name: "GetWidget".to_string(),
            file_name: "get_widget".to_string(),
            description: Some("Get a widget.".to_string()),
            deprecation: None,
            arg_fields: vec![make_field("id", "id", "String", true)],
            result_fields: vec![
                make_field("name", "name", "String", true),
                make_field("value", "value", "f64", true),
            ],
        };
        let code = emit_function_file(&function, "2.0.0");

        assert!(code.contains("pub struct GetWidgetArgs {"));
        assert!(code.contains("pub struct GetWidgetResult {"));
        assert!(code.contains("#[derive(pulumi::ProviderFunction)]"));
        assert!(code.contains("#[pulumi(token = \"test:index/getWidget:getWidget\")]"));
        assert!(code.contains("#[pulumi(args = GetWidgetArgs)]"));
        assert!(code.contains("#[pulumi(returns = GetWidgetResult)]"));
        assert!(code.contains("#[pulumi(version = \"2.0.0\")]"));
        assert!(code.contains("pub struct GetWidget;"));
    }

    // ---- Type files ----

    #[test]
    fn object_type_file() {
        let obj = ResolvedObject {
            token: "test:mod/Rule:Rule".to_string(),
            rust_name: "Rule".to_string(),
            module: "mod".to_string(),
            file_name: "rule".to_string(),
            description: Some("A rule.".to_string()),
            fields: vec![
                make_field("enabled", "enabled", "bool", true),
                make_field("prefix", "prefix", "Option<String>", false),
            ],
        };
        let code = emit_object_type(&obj);

        assert!(code.contains("#[derive(Serialize, Deserialize, Clone)]"));
        assert!(code.contains("pub struct Rule {"));
        assert!(code.contains("pub enabled: bool,"));
        assert!(code.contains(
            "#[serde(rename = \"prefix\", skip_serializing_if = \"Option::is_none\")]"
        ));
        assert!(code.contains("pub prefix: Option<String>,"));
    }

    #[test]
    fn enum_type_file() {
        let e = ResolvedEnum {
            token: "test:mod/Color:Color".to_string(),
            rust_name: "Color".to_string(),
            module: "mod".to_string(),
            file_name: "color".to_string(),
            description: Some("A color.".to_string()),
            underlying_type: "String".to_string(),
            variants: vec![
                ResolvedEnumVariant {
                    rust_name: "Red".to_string(),
                    value: "red".to_string(),
                    description: Some("The color red.".to_string()),
                    deprecation: None,
                },
                ResolvedEnumVariant {
                    rust_name: "Blue".to_string(),
                    value: "blue".to_string(),
                    description: None,
                    deprecation: None,
                },
            ],
        };
        let code = emit_enum_type(&e);

        assert!(code.contains("#[derive(Serialize, Deserialize, Clone, PartialEq)]"));
        assert!(code.contains("pub enum Color {"));
        assert!(code.contains("#[serde(rename = \"red\")]"));
        assert!(code.contains("    Red,"));
        assert!(code.contains("#[serde(rename = \"blue\")]"));
        assert!(code.contains("    Blue,"));
    }

    // ---- Union type file ----

    #[test]
    fn union_type_file() {
        let u = ResolvedUnion {
            rust_name: "StringOrInteger".to_string(),
            file_name: "string_or_integer".to_string(),
            variants: vec![
                crate::ir::UnionVariant { rust_name: "String".to_string(), rust_type: "String".to_string() },
                crate::ir::UnionVariant { rust_name: "Integer".to_string(), rust_type: "i64".to_string() },
            ],
        };
        let code = emit_union_type(&u);
        assert!(code.contains("#[derive(Serialize, Deserialize, Clone)]"));
        assert!(code.contains("#[serde(untagged)]"));
        assert!(code.contains("pub enum StringOrInteger {"));
        assert!(code.contains("    String(String),"));
        assert!(code.contains("    Integer(i64),"));
    }

    // ---- lib.rs ----

    #[test]
    fn lib_rs_root_only() {
        let module = crate::ir::ResolvedModule {
            resources: vec![ResolvedResource {
                type_token: String::new(),
                rust_name: "RandomId".to_string(),
                file_name: "random_id".to_string(),
                description: None,
                deprecation: None,
                is_component: false,
                input_fields: vec![],
                output_fields: vec![],
            }],
            functions: vec![],
        };
        let non_root: BTreeMap<&str, &crate::ir::ResolvedModule> = BTreeMap::new();
        let types: BTreeMap<String, Vec<&ResolvedType>> = BTreeMap::new();
        let code = emit_lib_rs(Some(&module), &non_root, &types);

        assert!(code.contains("mod random_id;"));
        assert!(code.contains("pub use random_id::*;"));
        assert!(!code.contains("pub mod types;"));
    }

    #[test]
    fn lib_rs_with_modules_and_types() {
        let root_module = crate::ir::ResolvedModule {
            resources: vec![],
            functions: vec![],
        };
        let s3_module = crate::ir::ResolvedModule {
            resources: vec![ResolvedResource {
                type_token: String::new(),
                rust_name: "Bucket".to_string(),
                file_name: "bucket".to_string(),
                description: None,
                deprecation: None,
                is_component: false,
                input_fields: vec![],
                output_fields: vec![],
            }],
            functions: vec![],
        };
        let mut non_root: BTreeMap<&str, &crate::ir::ResolvedModule> = BTreeMap::new();
        non_root.insert("s3", &s3_module);

        let obj = ResolvedType::Object(ResolvedObject {
            token: String::new(),
            rust_name: "Rule".to_string(),
            module: "s3".to_string(),
            file_name: "rule".to_string(),
            description: None,
            fields: vec![],
        });
        let mut types: BTreeMap<String, Vec<&ResolvedType>> = BTreeMap::new();
        types.insert("s3".to_string(), vec![&obj]);

        let code = emit_lib_rs(Some(&root_module), &non_root, &types);

        assert!(code.contains("pub mod types;"));
        assert!(code.contains("pub mod s3;"));
    }

    // ---- Doc comments and deprecation ----

    #[test]
    fn doc_comment_multiline() {
        let mut out = String::new();
        emit_doc_comment(&mut out, Some("Line one.\nLine two."));
        assert!(out.contains("/// Line one."));
        assert!(out.contains("/// Line two."));
    }

    #[test]
    fn deprecation_with_quotes() {
        let mut out = String::new();
        emit_deprecation_attr(&mut out, Some("Use \"New\" instead"));
        assert!(out.contains("#[deprecated(note = \"Use \\\"New\\\" instead\")]"));
    }

    // ---- Full emit_package to disk ----

    #[test]
    fn emit_package_to_disk() {
        let package = ResolvedPackage {
            name: "testpkg".to_string(),
            version: "0.1.0".to_string(),
            modules: {
                let mut m = BTreeMap::new();
                m.insert(
                    "".to_string(),
                    ResolvedModule {
                        resources: vec![ResolvedResource {
                            type_token: "testpkg:index/widget:Widget".to_string(),
                            rust_name: "Widget".to_string(),
                            file_name: "widget".to_string(),
                            description: Some("A widget.".to_string()),
                            deprecation: None,
                            is_component: false,
                            input_fields: vec![make_field("name", "name", "String", true)],
                            output_fields: vec![make_field("id", "id", "String", true)],
                        }],
                        functions: vec![],
                    },
                );
                m
            },
            types: BTreeMap::new(),
        };

        let dir = tempfile::tempdir().unwrap();
        emit_package(&package, dir.path()).unwrap();

        // Verify files exist.
        assert!(dir.path().join("Cargo.toml").exists());
        assert!(dir.path().join("src/lib.rs").exists());
        assert!(dir.path().join("src/widget.rs").exists());

        // Verify content.
        let cargo = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
        assert!(cargo.contains("pulumi-testpkg"));

        let lib = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
        assert!(lib.contains("mod widget;"));
        assert!(lib.contains("pub use widget::*;"));

        let widget = std::fs::read_to_string(dir.path().join("src/widget.rs")).unwrap();
        assert!(widget.contains("pub struct WidgetArgs {"));
        assert!(widget.contains("pub struct Widget;"));
    }
}
