//! Pulumi provider schema JSON deserialization types.
//!
//! These types mirror the structure defined in the Pulumi schema specification.
//! See: https://www.pulumi.com/docs/using-pulumi/pulumi-packages/schema/

use std::collections::BTreeMap;

use serde::Deserialize;

/// Top-level provider schema.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageSchema {
    pub name: String,
    pub version: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub meta: Option<Meta>,
    #[serde(default)]
    pub resources: BTreeMap<String, ResourceSpec>,
    #[serde(default)]
    pub functions: BTreeMap<String, FunctionSpec>,
    #[serde(default)]
    pub types: BTreeMap<String, ComplexTypeSpec>,
    pub provider: Option<ResourceSpec>,
    pub config: Option<ConfigSpec>,
    #[serde(default)]
    pub language: serde_json::Value,
}

/// Package metadata.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    /// Regex used to extract the module name from a type token.
    /// Default: `"(.*)(?:/[^/]*)"`
    pub module_format: Option<String>,
}

/// Configuration specification.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigSpec {
    #[serde(default)]
    pub variables: BTreeMap<String, PropertySpec>,
    #[serde(default)]
    pub required: Vec<String>,
}

/// A resource definition in the schema.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSpec {
    pub description: Option<String>,
    #[serde(default)]
    pub input_properties: BTreeMap<String, PropertySpec>,
    #[serde(default)]
    pub properties: BTreeMap<String, PropertySpec>,
    #[serde(default)]
    pub required_inputs: Vec<String>,
    #[serde(default)]
    pub required: Vec<String>,
    pub deprecation_message: Option<String>,
    #[serde(default)]
    pub is_component: bool,
    #[serde(default)]
    pub is_overlay: bool,
    #[serde(default)]
    pub methods: BTreeMap<String, String>,
    pub state_inputs: Option<ObjectTypeSpec>,
    #[serde(default)]
    pub aliases: Vec<AliasSpec>,
}

/// A function (invoke) definition in the schema.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionSpec {
    pub description: Option<String>,
    pub inputs: Option<ObjectTypeSpec>,
    pub outputs: Option<ObjectTypeSpec>,
    pub deprecation_message: Option<String>,
    #[serde(default)]
    pub is_overlay: bool,
    /// If present, this function is a method on the resource with this type token.
    pub multi_argument_inputs: Option<Vec<String>>,
}

/// An object type specification (used in function inputs/outputs and stateInputs).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectTypeSpec {
    #[serde(default)]
    pub properties: BTreeMap<String, PropertySpec>,
    #[serde(default)]
    pub required: Vec<String>,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub type_: Option<String>,
}

/// A complex type definition — either an object or an enum.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComplexTypeSpec {
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub type_: Option<String>,
    #[serde(default)]
    pub properties: BTreeMap<String, PropertySpec>,
    #[serde(default)]
    pub required: Vec<String>,
    /// If present, this is an enum type.
    #[serde(rename = "enum")]
    pub enum_values: Option<Vec<EnumValueSpec>>,
    #[serde(default)]
    pub is_overlay: bool,
}

impl ComplexTypeSpec {
    /// Returns true if this is an enum type (has enum values).
    pub fn is_enum(&self) -> bool {
        self.enum_values.is_some()
    }
}

/// A single enum variant.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnumValueSpec {
    pub name: Option<String>,
    pub value: serde_json::Value,
    pub description: Option<String>,
    pub deprecation_message: Option<String>,
}

/// A property (field) specification.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertySpec {
    // Type info (one of these patterns):
    #[serde(rename = "type")]
    pub type_: Option<String>,
    #[serde(rename = "$ref")]
    pub ref_: Option<String>,
    pub items: Option<Box<TypeSpec>>,
    pub additional_properties: Option<Box<TypeSpec>>,
    pub one_of: Option<Vec<TypeSpec>>,

    // Metadata:
    pub description: Option<String>,
    pub deprecation_message: Option<String>,
    #[serde(default)]
    pub secret: bool,
    #[serde(rename = "default")]
    pub default_value: Option<serde_json::Value>,
    #[serde(default)]
    pub plain: bool,
    #[serde(default)]
    pub replace_on_changes: bool,
    #[serde(default)]
    pub will_replace_on_changes: bool,
}

/// A type reference — used in `items`, `additionalProperties`, and `oneOf`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeSpec {
    #[serde(rename = "type")]
    pub type_: Option<String>,
    #[serde(rename = "$ref")]
    pub ref_: Option<String>,
    pub items: Option<Box<TypeSpec>>,
    pub additional_properties: Option<Box<TypeSpec>>,
    pub one_of: Option<Vec<TypeSpec>>,
    #[serde(default)]
    pub plain: bool,
}

/// A resource alias specification.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AliasSpec {
    pub name: Option<String>,
    pub project: Option<String>,
    #[serde(rename = "type")]
    pub type_: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_minimal_schema() {
        let json = r#"{
            "name": "test",
            "version": "1.0.0"
        }"#;
        let schema: PackageSchema = serde_json::from_str(json).unwrap();
        assert_eq!(schema.name, "test");
        assert_eq!(schema.version.as_deref(), Some("1.0.0"));
        assert!(schema.resources.is_empty());
        assert!(schema.functions.is_empty());
        assert!(schema.types.is_empty());
    }

    #[test]
    fn deserialize_resource_with_properties() {
        let json = r#"{
            "name": "test",
            "resources": {
                "test:index:MyResource": {
                    "inputProperties": {
                        "name": {
                            "type": "string",
                            "description": "The name of the resource"
                        },
                        "count": {
                            "type": "integer"
                        }
                    },
                    "properties": {
                        "id": {
                            "type": "string"
                        },
                        "name": {
                            "type": "string"
                        }
                    },
                    "requiredInputs": ["name"],
                    "required": ["id", "name"]
                }
            }
        }"#;
        let schema: PackageSchema = serde_json::from_str(json).unwrap();
        let res = &schema.resources["test:index:MyResource"];
        assert_eq!(res.input_properties.len(), 2);
        assert_eq!(res.properties.len(), 2);
        assert_eq!(res.required_inputs, vec!["name"]);
        assert_eq!(res.required, vec!["id", "name"]);
    }

    #[test]
    fn deserialize_function() {
        let json = r#"{
            "name": "test",
            "functions": {
                "test:index:getWidget": {
                    "description": "Get a widget",
                    "inputs": {
                        "properties": {
                            "id": { "type": "string" }
                        },
                        "required": ["id"]
                    },
                    "outputs": {
                        "properties": {
                            "name": { "type": "string" },
                            "value": { "type": "number" }
                        },
                        "required": ["name", "value"]
                    }
                }
            }
        }"#;
        let schema: PackageSchema = serde_json::from_str(json).unwrap();
        let func = &schema.functions["test:index:getWidget"];
        assert_eq!(func.description.as_deref(), Some("Get a widget"));
        let inputs = func.inputs.as_ref().unwrap();
        assert_eq!(inputs.properties.len(), 1);
        let outputs = func.outputs.as_ref().unwrap();
        assert_eq!(outputs.properties.len(), 2);
    }

    #[test]
    fn deserialize_complex_type_object() {
        let json = r#"{
            "name": "test",
            "types": {
                "test:index:LifecycleRule": {
                    "type": "object",
                    "properties": {
                        "enabled": { "type": "boolean" },
                        "prefix": { "type": "string" }
                    },
                    "required": ["enabled"]
                }
            }
        }"#;
        let schema: PackageSchema = serde_json::from_str(json).unwrap();
        let ty = &schema.types["test:index:LifecycleRule"];
        assert!(!ty.is_enum());
        assert_eq!(ty.properties.len(), 2);
        assert_eq!(ty.required, vec!["enabled"]);
    }

    #[test]
    fn deserialize_complex_type_enum() {
        let json = r#"{
            "name": "test",
            "types": {
                "test:index:Color": {
                    "type": "string",
                    "enum": [
                        { "name": "Red", "value": "red" },
                        { "name": "Green", "value": "green", "description": "The color green" },
                        { "value": "blue" }
                    ]
                }
            }
        }"#;
        let schema: PackageSchema = serde_json::from_str(json).unwrap();
        let ty = &schema.types["test:index:Color"];
        assert!(ty.is_enum());
        let variants = ty.enum_values.as_ref().unwrap();
        assert_eq!(variants.len(), 3);
        assert_eq!(variants[0].name.as_deref(), Some("Red"));
        assert_eq!(variants[0].value, serde_json::Value::String("red".into()));
        assert_eq!(variants[2].name, None);
    }

    #[test]
    fn deserialize_ref_and_array_types() {
        let json = r##"{
            "name": "test",
            "resources": {
                "test:index:MyResource": {
                    "inputProperties": {
                        "tags": {
                            "type": "object",
                            "additionalProperties": { "type": "string" }
                        },
                        "rules": {
                            "type": "array",
                            "items": { "$ref": "#/types/test:index:Rule" }
                        },
                        "config": {
                            "$ref": "#/types/test:index:Config"
                        },
                        "any_value": {
                            "$ref": "pulumi.json#/Any"
                        }
                    }
                }
            }
        }"##;
        let schema: PackageSchema = serde_json::from_str(json).unwrap();
        let res = &schema.resources["test:index:MyResource"];
        let props = &res.input_properties;

        // Map type
        let tags = &props["tags"];
        assert_eq!(tags.type_.as_deref(), Some("object"));
        assert!(tags.additional_properties.is_some());

        // Array of refs
        let rules = &props["rules"];
        assert_eq!(rules.type_.as_deref(), Some("array"));
        let items = rules.items.as_ref().unwrap();
        assert_eq!(items.ref_.as_deref(), Some("#/types/test:index:Rule"));

        // Direct ref
        let config = &props["config"];
        assert_eq!(config.ref_.as_deref(), Some("#/types/test:index:Config"));

        // Pulumi built-in ref
        let any = &props["any_value"];
        assert_eq!(any.ref_.as_deref(), Some("pulumi.json#/Any"));
    }

    #[test]
    fn deserialize_one_of() {
        let json = r#"{
            "name": "test",
            "resources": {
                "test:index:MyResource": {
                    "inputProperties": {
                        "value": {
                            "oneOf": [
                                { "type": "string" },
                                { "type": "integer" }
                            ]
                        }
                    }
                }
            }
        }"#;
        let schema: PackageSchema = serde_json::from_str(json).unwrap();
        let res = &schema.resources["test:index:MyResource"];
        let val = &res.input_properties["value"];
        let one_of = val.one_of.as_ref().unwrap();
        assert_eq!(one_of.len(), 2);
        assert_eq!(one_of[0].type_.as_deref(), Some("string"));
        assert_eq!(one_of[1].type_.as_deref(), Some("integer"));
    }

    #[test]
    fn deserialize_meta_and_deprecation() {
        let json = r#"{
            "name": "test",
            "meta": {
                "moduleFormat": "(.*)"
            },
            "resources": {
                "test:index:Old": {
                    "deprecationMessage": "Use NewResource instead",
                    "isComponent": true,
                    "inputProperties": {
                        "oldField": {
                            "type": "string",
                            "deprecationMessage": "Use newField instead",
                            "secret": true,
                            "replaceOnChanges": true
                        }
                    }
                }
            }
        }"#;
        let schema: PackageSchema = serde_json::from_str(json).unwrap();
        assert_eq!(
            schema.meta.as_ref().unwrap().module_format.as_deref(),
            Some("(.*)")
        );
        let res = &schema.resources["test:index:Old"];
        assert_eq!(
            res.deprecation_message.as_deref(),
            Some("Use NewResource instead")
        );
        assert!(res.is_component);
        let field = &res.input_properties["oldField"];
        assert!(field.secret);
        assert!(field.replace_on_changes);
        assert_eq!(
            field.deprecation_message.as_deref(),
            Some("Use newField instead")
        );
    }

    #[test]
    fn deserialize_aliases() {
        let json = r#"{
            "name": "test",
            "resources": {
                "test:index:MyResource": {
                    "aliases": [
                        { "type": "test:index:OldName" },
                        { "name": "old-name" }
                    ]
                }
            }
        }"#;
        let schema: PackageSchema = serde_json::from_str(json).unwrap();
        let res = &schema.resources["test:index:MyResource"];
        assert_eq!(res.aliases.len(), 2);
        assert_eq!(
            res.aliases[0].type_.as_deref(),
            Some("test:index:OldName")
        );
        assert_eq!(res.aliases[1].name.as_deref(), Some("old-name"));
    }
}
