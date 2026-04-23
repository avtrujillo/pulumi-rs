/// Asset and Archive types for Pulumi resources.
///
/// Assets represent file contents (inline text, a URI, or a local path).
/// Archives represent collections of assets or a single archive file.

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum Asset {
    Text { text: String },
    Uri { uri: String },
    Path { path: String },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum Archive {
    Assets {
        assets: std::collections::HashMap<String, Asset>,
    },
    Uri {
        uri: String,
    },
    Path {
        path: String,
    },
}
