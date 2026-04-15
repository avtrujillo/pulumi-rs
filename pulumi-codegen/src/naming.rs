//! Name conversion utilities for Pulumi schema → Rust code generation.
//!
//! Handles camelCase→snake_case conversion, PascalCase conversion,
//! Rust keyword escaping, and Pulumi type token parsing.

use regex::Regex;

/// Convert a camelCase or PascalCase identifier to snake_case.
///
/// Handles acronyms correctly:
/// - `bucketPrefix` → `bucket_prefix`
/// - `vpcId` → `vpc_id`
/// - `SHA256Hash` → `sha256_hash`
/// - `s3BucketArn` → `s3_bucket_arn`
/// - `ec2Instance` → `ec2_instance`
/// - `getHTTPSPort` → `get_https_port`
pub fn camel_to_snake_case(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    let mut result = String::with_capacity(s.len() + 4);
    let chars: Vec<char> = s.chars().collect();

    for i in 0..chars.len() {
        let c = chars[i];

        if i == 0 {
            result.push(c.to_ascii_lowercase());
            continue;
        }

        if c.is_ascii_uppercase() {
            let prev = chars[i - 1];
            let next = chars.get(i + 1);

            if prev.is_ascii_lowercase() || prev.is_ascii_digit() {
                // Transition from lowercase/digit to uppercase: `bucketP` → `bucket_p`
                result.push('_');
            } else if prev.is_ascii_uppercase() {
                // In an acronym run. Only insert underscore if next char is lowercase,
                // indicating end of acronym: `SH` in `SHA256` stays together,
                // but `AH` in `SHA256Hash` → `...A_H` wait no.
                // Actually: `SHA256Hash` - when we're at 'H' of 'Hash', prev is '6' (digit)
                // so that's handled above. Let's think about `getHTTPSPort`:
                // g-e-t-H-T-T-P-S-P-o-r-t
                // At 'P' (index 8): prev='S' (upper), next='o' (lower) → insert '_'
                // At 'S' (index 7): prev='P' (upper), next='P' (upper) → no insert
                // Result: get_https_port ✓
                if let Some(&next_c) = next {
                    if next_c.is_ascii_lowercase() {
                        result.push('_');
                    }
                }
            }
            result.push(c.to_ascii_lowercase());
        } else if c.is_ascii_digit() && !chars[i - 1].is_ascii_digit() && !result.ends_with('_') {
            // Transition from letter to digit: `sha256` - we want `sha256` not `sha_256`
            // Actually for `s3Bucket` we want `s3_bucket`, digits stay with preceding letters.
            // But `SHA256Hash`: we do NOT insert underscore between A and 2.
            // Rule: don't insert underscore before digits that follow letters within a "word".
            result.push(c);
        } else {
            result.push(c);
        }
    }

    result
}

/// Convert a string to PascalCase.
///
/// Handles inputs that are camelCase, snake_case, kebab-case, or already PascalCase.
/// - `randomId` → `RandomId`
/// - `random_id` → `RandomId`
/// - `public-read` → `PublicRead`
/// - `RandomId` → `RandomId`
pub fn to_pascal_case(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    let mut result = String::with_capacity(s.len());
    let mut capitalize_next = true;

    for c in s.chars() {
        if c == '_' || c == '-' || c == '.' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }

    result
}

/// Rust keywords that support raw identifier syntax (`r#keyword`).
const RAW_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut",
    "pub", "ref", "return", "static", "struct", "trait", "true", "type", "union", "unsafe",
    "use", "where", "while", "yield", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "try", "typeof", "unsized", "virtual",
];

/// Rust keywords that do NOT support `r#` syntax. These get a trailing underscore.
const SPECIAL_KEYWORDS: &[&str] = &["self", "crate", "super", "Self"];

/// Escape a Rust keyword if necessary.
///
/// - Most keywords → `r#keyword`
/// - `self`, `crate`, `super`, `Self` → `self_`, `crate_`, `super_`, `Self_`
/// - Non-keywords → returned unchanged
pub fn escape_rust_keyword(name: &str) -> String {
    if SPECIAL_KEYWORDS.contains(&name) {
        format!("{name}_")
    } else if RAW_KEYWORDS.contains(&name) {
        format!("r#{name}")
    } else {
        name.to_string()
    }
}

/// Result of parsing a Pulumi type token.
#[derive(Debug, PartialEq)]
pub struct ParsedToken {
    /// The package name (e.g., `aws`).
    pub package: String,
    /// The module name (e.g., `s3`), or empty string for the root/index module.
    pub module: String,
    /// The type name (e.g., `Bucket`).
    pub name: String,
}

/// Parse a Pulumi type token into its components.
///
/// Tokens have the format `pkg:module/subpath:Name`. The `module_format` regex
/// (from the schema's `meta.moduleFormat`) extracts the module from the
/// `module/subpath` segment. The first capture group is the module name.
///
/// Default `module_format`: `(.*)(?:/[^/]*)`
///
/// Examples:
/// - `aws:s3/bucket:Bucket` with default format → `("aws", "s3", "Bucket")`
/// - `random:index/randomId:RandomId` with default format → `("random", "", "RandomId")`
///   (index maps to the root module, represented as empty string)
pub fn parse_type_token(token: &str, module_format: Option<&str>) -> Option<ParsedToken> {
    // Split on the first and last `:` — format is `pkg:module_path:Name`
    let first_colon = token.find(':')?;
    let last_colon = token.rfind(':')?;
    if first_colon == last_colon {
        return None;
    }

    let package = &token[..first_colon];
    let module_path = &token[first_colon + 1..last_colon];
    let name = &token[last_colon + 1..];

    // Apply moduleFormat regex to extract the module from module_path.
    let module = extract_module(module_path, module_format);

    // "index" maps to root module (empty string).
    let module = if module == "index" {
        String::new()
    } else {
        module
    };

    Some(ParsedToken {
        package: package.to_string(),
        module,
        name: name.to_string(),
    })
}

/// Extract the module name from a module path using the moduleFormat regex.
fn extract_module(module_path: &str, module_format: Option<&str>) -> String {
    let format = module_format.unwrap_or("(.*)(?:/[^/]*)");
    if let Ok(re) = Regex::new(format) {
        if let Some(caps) = re.captures(module_path) {
            if let Some(m) = caps.get(1) {
                return m.as_str().to_string();
            }
        }
    }
    // If the regex doesn't match (e.g., simple token with no `/`), use the whole path.
    module_path.to_string()
}

/// Convert a module name to a valid Rust module identifier.
///
/// - Convert to snake_case
/// - Replace `.` with `_`
/// - Escape Rust keywords
pub fn module_to_rust_identifier(module: &str) -> String {
    if module.is_empty() {
        return String::new();
    }
    let sanitized = module.replace('.', "_");
    let snake = camel_to_snake_case(&sanitized);
    escape_rust_keyword(&snake)
}

/// Convert a type name to a file name (snake_case, no extension).
pub fn type_name_to_file_name(name: &str) -> String {
    camel_to_snake_case(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- camel_to_snake_case tests ----

    #[test]
    fn snake_basic() {
        assert_eq!(camel_to_snake_case("bucketPrefix"), "bucket_prefix");
    }

    #[test]
    fn snake_single_word() {
        assert_eq!(camel_to_snake_case("bucket"), "bucket");
    }

    #[test]
    fn snake_already_snake() {
        assert_eq!(camel_to_snake_case("bucket_prefix"), "bucket_prefix");
    }

    #[test]
    fn snake_pascal_case() {
        assert_eq!(camel_to_snake_case("BucketPrefix"), "bucket_prefix");
    }

    #[test]
    fn snake_acronym_end() {
        assert_eq!(camel_to_snake_case("vpcId"), "vpc_id");
    }

    #[test]
    fn snake_acronym_start() {
        assert_eq!(camel_to_snake_case("SHA256Hash"), "sha256_hash");
    }

    #[test]
    fn snake_leading_digits() {
        assert_eq!(camel_to_snake_case("s3Bucket"), "s3_bucket");
    }

    #[test]
    fn snake_leading_digits_2() {
        assert_eq!(camel_to_snake_case("ec2Instance"), "ec2_instance");
    }

    #[test]
    fn snake_multi_acronym() {
        assert_eq!(camel_to_snake_case("s3BucketArn"), "s3_bucket_arn");
    }

    #[test]
    fn snake_all_caps() {
        assert_eq!(camel_to_snake_case("URL"), "url");
    }

    #[test]
    fn snake_acronym_then_word() {
        assert_eq!(camel_to_snake_case("getHTTPSPort"), "get_https_port");
    }

    #[test]
    fn snake_empty() {
        assert_eq!(camel_to_snake_case(""), "");
    }

    #[test]
    fn snake_numbers_in_middle() {
        assert_eq!(camel_to_snake_case("base64Encoded"), "base64_encoded");
    }

    #[test]
    fn snake_consecutive_uppers_then_lower() {
        assert_eq!(camel_to_snake_case("XMLParser"), "xml_parser");
    }

    #[test]
    fn snake_id_suffix() {
        assert_eq!(camel_to_snake_case("resourceId"), "resource_id");
    }

    // ---- to_pascal_case tests ----

    #[test]
    fn pascal_from_camel() {
        assert_eq!(to_pascal_case("randomId"), "RandomId");
    }

    #[test]
    fn pascal_from_snake() {
        assert_eq!(to_pascal_case("random_id"), "RandomId");
    }

    #[test]
    fn pascal_from_kebab() {
        assert_eq!(to_pascal_case("public-read"), "PublicRead");
    }

    #[test]
    fn pascal_already() {
        assert_eq!(to_pascal_case("RandomId"), "RandomId");
    }

    #[test]
    fn pascal_empty() {
        assert_eq!(to_pascal_case(""), "");
    }

    #[test]
    fn pascal_single_char() {
        assert_eq!(to_pascal_case("a"), "A");
    }

    #[test]
    fn pascal_from_dotted() {
        assert_eq!(to_pascal_case("public.read.write"), "PublicReadWrite");
    }

    // ---- escape_rust_keyword tests ----

    #[test]
    fn escape_type() {
        assert_eq!(escape_rust_keyword("type"), "r#type");
    }

    #[test]
    fn escape_self() {
        assert_eq!(escape_rust_keyword("self"), "self_");
    }

    #[test]
    fn escape_crate() {
        assert_eq!(escape_rust_keyword("crate"), "crate_");
    }

    #[test]
    fn escape_super() {
        assert_eq!(escape_rust_keyword("super"), "super_");
    }

    #[test]
    fn escape_upper_self() {
        assert_eq!(escape_rust_keyword("Self"), "Self_");
    }

    #[test]
    fn escape_non_keyword() {
        assert_eq!(escape_rust_keyword("bucket"), "bucket");
    }

    #[test]
    fn escape_async() {
        assert_eq!(escape_rust_keyword("async"), "r#async");
    }

    #[test]
    fn escape_match() {
        assert_eq!(escape_rust_keyword("match"), "r#match");
    }

    // ---- parse_type_token tests ----

    #[test]
    fn token_aws_resource() {
        let result = parse_type_token("aws:s3/bucket:Bucket", None).unwrap();
        assert_eq!(result, ParsedToken {
            package: "aws".into(),
            module: "s3".into(),
            name: "Bucket".into(),
        });
    }

    #[test]
    fn token_index_module() {
        let result = parse_type_token("random:index/randomId:RandomId", None).unwrap();
        assert_eq!(result, ParsedToken {
            package: "random".into(),
            module: "".into(),
            name: "RandomId".into(),
        });
    }

    #[test]
    fn token_nested_module() {
        let result = parse_type_token("aws:ec2/instance:Instance", None).unwrap();
        assert_eq!(result, ParsedToken {
            package: "aws".into(),
            module: "ec2".into(),
            name: "Instance".into(),
        });
    }

    #[test]
    fn token_function() {
        let result = parse_type_token("aws:s3/getBucket:getBucket", None).unwrap();
        assert_eq!(result, ParsedToken {
            package: "aws".into(),
            module: "s3".into(),
            name: "getBucket".into(),
        });
    }

    #[test]
    fn token_invalid_no_colon() {
        assert!(parse_type_token("invalid", None).is_none());
    }

    #[test]
    fn token_invalid_single_colon() {
        assert!(parse_type_token("pkg:rest", None).is_none());
    }

    #[test]
    fn token_custom_module_format() {
        // Module format that just takes everything before `/`
        let result =
            parse_type_token("k8s:admissionregistration.k8s.io/v1:Webhook", Some("(.*)/(.*)"))
                .unwrap();
        assert_eq!(result, ParsedToken {
            package: "k8s".into(),
            module: "admissionregistration.k8s.io".into(),
            name: "Webhook".into(),
        });
    }

    // ---- module_to_rust_identifier tests ----

    #[test]
    fn module_simple() {
        assert_eq!(module_to_rust_identifier("s3"), "s3");
    }

    #[test]
    fn module_camel_case() {
        assert_eq!(
            module_to_rust_identifier("applicationLoadBalancing"),
            "application_load_balancing"
        );
    }

    #[test]
    fn module_dotted() {
        assert_eq!(
            module_to_rust_identifier("admissionregistration.k8s.io"),
            "admissionregistration_k8s_io"
        );
    }

    #[test]
    fn module_empty() {
        assert_eq!(module_to_rust_identifier(""), "");
    }

    #[test]
    fn module_keyword() {
        assert_eq!(module_to_rust_identifier("type"), "r#type");
    }

    // ---- type_name_to_file_name tests ----

    #[test]
    fn file_name_simple() {
        assert_eq!(type_name_to_file_name("Bucket"), "bucket");
    }

    #[test]
    fn file_name_multi_word() {
        assert_eq!(type_name_to_file_name("BucketObject"), "bucket_object");
    }

    #[test]
    fn file_name_acronym() {
        assert_eq!(type_name_to_file_name("RandomId"), "random_id");
    }

    #[test]
    fn file_name_camel() {
        assert_eq!(type_name_to_file_name("getBucket"), "get_bucket");
    }
}
