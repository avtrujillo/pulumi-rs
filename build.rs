use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Locate protoc's well-known include directory (for google/protobuf/*.proto).
    // Protoc is typically installed at <prefix>/bin/protoc with includes at <prefix>/include.
    let protoc = std::env::var("PROTOC")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("protoc"));

    let mut include_dirs: Vec<PathBuf> = vec!["proto".into()];

    // Try <protoc_dir>/../include (works for standard installations and GitHub releases).
    if let Some(bin_dir) = protoc.parent() {
        if let Some(prefix) = bin_dir.parent() {
            let inc = prefix.join("include");
            if inc.join("google/protobuf/struct.proto").exists() {
                include_dirs.push(inc);
            }
        }
    }

    // Also try the which/where result if PROTOC wasn't set.
    if include_dirs.len() == 1 {
        if let Ok(output) = std::process::Command::new("which").arg("protoc").output() {
            if let Ok(path) = std::str::from_utf8(&output.stdout) {
                let path = PathBuf::from(path.trim());
                if let Some(prefix) = path.parent().and_then(|p| p.parent()) {
                    let inc = prefix.join("include");
                    if inc.join("google/protobuf/struct.proto").exists() {
                        include_dirs.push(inc);
                    }
                }
            }
        }
    }

    tonic_build::configure()
        .build_server(false)
        .compile_protos(
            &[
                "proto/pulumi/resource.proto",
                "proto/pulumi/engine.proto",
                "proto/pulumi/provider.proto",
                "proto/pulumi/callback.proto",
            ],
            &include_dirs,
        )?;
    Ok(())
}
