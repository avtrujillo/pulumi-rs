use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Parser;

/// Generate a typed Rust crate from a Pulumi provider schema JSON file.
#[derive(Parser)]
#[command(name = "pulumi-codegen", version)]
struct Cli {
    /// Path to the Pulumi provider schema JSON file.
    #[arg(long)]
    schema: PathBuf,

    /// Output directory for the generated crate.
    #[arg(long)]
    out: PathBuf,

    /// Override the version in the generated Cargo.toml.
    #[arg(long)]
    version_override: Option<String>,

    /// Run rustfmt on the generated code.
    #[arg(long)]
    format: bool,
}

fn main() {
    let cli = Cli::parse();

    let json = std::fs::read_to_string(&cli.schema).unwrap_or_else(|e| {
        eprintln!("Error reading schema file {:?}: {e}", cli.schema);
        std::process::exit(1);
    });

    let schema: pulumi_codegen::schema::PackageSchema =
        serde_json::from_str(&json).unwrap_or_else(|e| {
            eprintln!("Error parsing schema JSON: {e}");
            std::process::exit(1);
        });

    let mut package = pulumi_codegen::ir::resolve_package(&schema);

    if let Some(version) = cli.version_override {
        package.version = version;
    }

    pulumi_codegen::emit::emit_package(&package, &cli.out).unwrap_or_else(|e| {
        eprintln!("Error writing generated crate to {:?}: {e}", cli.out);
        std::process::exit(1);
    });

    let resource_count: usize = package.modules.values().map(|m| m.resources.len()).sum();
    let function_count: usize = package.modules.values().map(|m| m.functions.len()).sum();
    let type_count = package.types.len();

    eprintln!(
        "Generated pulumi-{} v{} -> {:?} ({} resources, {} functions, {} types)",
        package.name,
        package.version,
        cli.out,
        resource_count,
        function_count,
        type_count,
    );

    if cli.format {
        let mut rs_files = Vec::new();
        collect_rs_files(&cli.out.join("src"), &mut rs_files);

        if !rs_files.is_empty() {
            let status = Command::new("rustfmt")
                .arg("--edition")
                .arg("2024")
                .args(&rs_files)
                .status();

            match status {
                Ok(s) if s.success() => eprintln!("Formatted with rustfmt."),
                Ok(s) => eprintln!("rustfmt exited with {s}"),
                Err(e) => eprintln!("Failed to run rustfmt: {e}"),
            }
        }
    }
}

fn collect_rs_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}
