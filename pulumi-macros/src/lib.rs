//! # Pulumi Derive Macros
//!
//! Procedural macros for reducing boilerplate when defining Pulumi resources
//! and provider functions.
//!
//! These macros are re-exported by the `pulumi` crate when the `macros` feature
//! is enabled. You typically don't need to depend on this crate directly.
//!
//! ## Available Macros
//!
//! - [`Resource`] — Derive the `Resource` trait for a custom cloud resource.
//! - [`ComponentResource`] — Derive the `ComponentResource` trait for a logical grouping.
//! - [`ProviderFunction`] — Derive the `ProviderFunction` trait for a read-only provider function.

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

/// Derives the `Resource` trait for a custom cloud resource.
///
/// # Required Attributes
///
/// - `#[pulumi(type_token = "...")]` — The Pulumi type token (e.g. `"aws:s3/bucket:Bucket"`)
/// - `#[pulumi(inputs = SomeInputType)]` — The input properties type (must impl `Serialize`)
/// - `#[pulumi(outputs = SomeOutputType)]` — The output properties type (must impl `DeserializeOwned + Clone`)
///
/// # Optional Attributes
///
/// - `#[pulumi(version = "...")]` — Provider plugin version
/// - `#[pulumi(plugin_download_url = "...")]` — Provider plugin download URL
///
/// # Example
///
/// ```ignore
/// use pulumi::Resource;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Serialize)]
/// struct BucketArgs {
///     bucket: String,
/// }
///
/// #[derive(Deserialize, Clone)]
/// struct BucketOutputs {
///     arn: String,
///     bucket: String,
/// }
///
/// #[derive(Resource)]
/// #[pulumi(type_token = "aws:s3/bucket:Bucket")]
/// #[pulumi(inputs = BucketArgs)]
/// #[pulumi(outputs = BucketOutputs)]
/// struct Bucket;
/// ```
#[proc_macro_derive(Resource, attributes(pulumi))]
pub fn derive_resource(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let name = &input.ident;
    let attrs = match parse_pulumi_attrs(&input) {
        Ok(a) => a,
        Err(e) => return e.to_compile_error().into(),
    };

    let type_token = match &attrs.type_token {
        Some(t) => t.clone(),
        None => {
            return syn::Error::new_spanned(
                &input.ident,
                "missing #[pulumi(type_token = \"...\")]",
            )
            .to_compile_error()
            .into();
        }
    };

    let inputs_ty = match &attrs.inputs {
        Some(t) => t.clone(),
        None => {
            return syn::Error::new_spanned(&input.ident, "missing #[pulumi(inputs = Type)]")
                .to_compile_error()
                .into();
        }
    };

    let outputs_ty = match &attrs.outputs {
        Some(t) => t.clone(),
        None => {
            return syn::Error::new_spanned(&input.ident, "missing #[pulumi(outputs = Type)]")
                .to_compile_error()
                .into();
        }
    };

    let version = attrs.version.as_deref().unwrap_or("");
    let plugin_download_url = attrs.plugin_download_url.as_deref().unwrap_or("");

    let expanded = quote! {
        impl ::pulumi_core::resource::Resource for #name {
            const TYPE_TOKEN: &'static str = #type_token;
            const VERSION: &'static str = #version;
            const PLUGIN_DOWNLOAD_URL: &'static str = #plugin_download_url;
            type Inputs = #inputs_ty;
            type Outputs = #outputs_ty;
        }
    };

    expanded.into()
}

/// Derives the `ComponentResource` trait for a logical grouping resource.
///
/// # Required Attributes
///
/// - `#[pulumi(type_token = "...")]` — The Pulumi type token (e.g. `"my:module:MyComponent"`)
///
/// # Example
///
/// ```ignore
/// use pulumi::ComponentResource;
///
/// #[derive(ComponentResource)]
/// #[pulumi(type_token = "my:module:WebApp")]
/// struct WebApp;
/// ```
#[proc_macro_derive(ComponentResource, attributes(pulumi))]
pub fn derive_component_resource(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let name = &input.ident;
    let attrs = match parse_pulumi_attrs(&input) {
        Ok(a) => a,
        Err(e) => return e.to_compile_error().into(),
    };

    let type_token = match &attrs.type_token {
        Some(t) => t.clone(),
        None => {
            return syn::Error::new_spanned(
                &input.ident,
                "missing #[pulumi(type_token = \"...\")]",
            )
            .to_compile_error()
            .into();
        }
    };

    let expanded = quote! {
        impl ::pulumi_core::resource::ComponentResource for #name {
            const TYPE_TOKEN: &'static str = #type_token;
        }
    };

    expanded.into()
}

/// Derives the `ProviderFunction` trait for a read-only provider function.
///
/// # Required Attributes
///
/// - `#[pulumi(token = "...")]` — The function token (e.g. `"aws:index/getAmi:getAmi"`)
/// - `#[pulumi(args = SomeArgsType)]` — The arguments type (must impl `Serialize`)
/// - `#[pulumi(returns = SomeReturnType)]` — The return type (must impl `DeserializeOwned`)
///
/// # Optional Attributes
///
/// - `#[pulumi(version = "...")]` — Provider plugin version
/// - `#[pulumi(plugin_download_url = "...")]` — Provider plugin download URL
///
/// # Example
///
/// ```ignore
/// use pulumi::ProviderFunction;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Serialize)]
/// struct GetAmiArgs {
///     most_recent: bool,
///     owners: Vec<String>,
/// }
///
/// #[derive(Deserialize)]
/// struct GetAmiResult {
///     id: String,
///     architecture: String,
/// }
///
/// #[derive(ProviderFunction)]
/// #[pulumi(token = "aws:index/getAmi:getAmi")]
/// #[pulumi(args = GetAmiArgs)]
/// #[pulumi(returns = GetAmiResult)]
/// struct GetAmi;
/// ```
#[proc_macro_derive(ProviderFunction, attributes(pulumi))]
pub fn derive_provider_function(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let name = &input.ident;
    let attrs = match parse_pulumi_attrs(&input) {
        Ok(a) => a,
        Err(e) => return e.to_compile_error().into(),
    };

    let token = match &attrs.token {
        Some(t) => t.clone(),
        None => {
            return syn::Error::new_spanned(&input.ident, "missing #[pulumi(token = \"...\")]")
                .to_compile_error()
                .into();
        }
    };

    let args_ty = match &attrs.args {
        Some(t) => t.clone(),
        None => {
            return syn::Error::new_spanned(&input.ident, "missing #[pulumi(args = Type)]")
                .to_compile_error()
                .into();
        }
    };

    let returns_ty = match &attrs.returns {
        Some(t) => t.clone(),
        None => {
            return syn::Error::new_spanned(&input.ident, "missing #[pulumi(returns = Type)]")
                .to_compile_error()
                .into();
        }
    };

    let version = attrs.version.as_deref().unwrap_or("");
    let plugin_download_url = attrs.plugin_download_url.as_deref().unwrap_or("");

    let expanded = quote! {
        impl ::pulumi_core::invoke::ProviderFunction for #name {
            const TOKEN: &'static str = #token;
            const VERSION: &'static str = #version;
            const PLUGIN_DOWNLOAD_URL: &'static str = #plugin_download_url;
            type Args = #args_ty;
            type Returns = #returns_ty;
        }
    };

    expanded.into()
}

// ---------------------------------------------------------------------------
// Attribute parsing
// ---------------------------------------------------------------------------

/// Parsed `#[pulumi(...)]` attributes.
#[derive(Default)]
struct PulumiAttrs {
    // Resource
    type_token: Option<String>,
    inputs: Option<syn::Type>,
    outputs: Option<syn::Type>,

    // ProviderFunction
    token: Option<String>,
    args: Option<syn::Type>,
    returns: Option<syn::Type>,

    // Shared
    version: Option<String>,
    plugin_download_url: Option<String>,
}

fn parse_pulumi_attrs(input: &DeriveInput) -> syn::Result<PulumiAttrs> {
    let mut attrs = PulumiAttrs::default();

    for attr in &input.attrs {
        if !attr.path().is_ident("pulumi") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("type_token") {
                let value = meta.value()?;
                let lit: syn::LitStr = value.parse()?;
                attrs.type_token = Some(lit.value());
            } else if meta.path.is_ident("token") {
                let value = meta.value()?;
                let lit: syn::LitStr = value.parse()?;
                attrs.token = Some(lit.value());
            } else if meta.path.is_ident("version") {
                let value = meta.value()?;
                let lit: syn::LitStr = value.parse()?;
                attrs.version = Some(lit.value());
            } else if meta.path.is_ident("plugin_download_url") {
                let value = meta.value()?;
                let lit: syn::LitStr = value.parse()?;
                attrs.plugin_download_url = Some(lit.value());
            } else if meta.path.is_ident("inputs") {
                let value = meta.value()?;
                let ty: syn::Type = value.parse()?;
                attrs.inputs = Some(ty);
            } else if meta.path.is_ident("outputs") {
                let value = meta.value()?;
                let ty: syn::Type = value.parse()?;
                attrs.outputs = Some(ty);
            } else if meta.path.is_ident("args") {
                let value = meta.value()?;
                let ty: syn::Type = value.parse()?;
                attrs.args = Some(ty);
            } else if meta.path.is_ident("returns") {
                let value = meta.value()?;
                let ty: syn::Type = value.parse()?;
                attrs.returns = Some(ty);
            } else {
                return Err(meta.error(format!(
                    "unknown pulumi attribute: {}",
                    meta.path
                        .get_ident()
                        .map(|i| i.to_string())
                        .unwrap_or_default()
                )));
            }
            Ok(())
        })?;
    }

    Ok(attrs)
}
