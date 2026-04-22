//! # pulumi-rs CLI
//!
//! A Rust-native CLI for Pulumi stack operations, built on `pulumi-automation`.

use clap::{Parser, Subcommand};
use colored::Colorize;
use pulumi_automation::config::ConfigValue;
use pulumi_automation::event::EngineEvent;
use pulumi_automation::LocalWorkspace;

#[derive(Parser)]
#[command(
    name = "pulumi-rs",
    about = "Rust-native CLI for Pulumi stack operations"
)]
struct Cli {
    /// Path to the Pulumi project directory.
    #[arg(short, long, default_value = ".")]
    cwd: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create or update resources in a stack.
    Up {
        /// The stack to operate on.
        #[arg(short, long)]
        stack: String,
        /// Emit output as JSON.
        #[arg(long)]
        json: bool,
        /// Use the native Rust engine instead of the Pulumi CLI.
        #[arg(long)]
        native: bool,
        /// Program binary to run (required with --native).
        #[arg(long)]
        program: Option<String>,
    },
    /// Show a preview of pending changes.
    Preview {
        /// The stack to operate on.
        #[arg(short, long)]
        stack: String,
        /// Emit output as JSON.
        #[arg(long)]
        json: bool,
        /// Use the native Rust engine instead of the Pulumi CLI.
        #[arg(long)]
        native: bool,
        /// Program binary to run (required with --native).
        #[arg(long)]
        program: Option<String>,
    },
    /// Destroy all resources in a stack.
    Destroy {
        /// The stack to operate on.
        #[arg(short, long)]
        stack: String,
        /// Emit output as JSON.
        #[arg(long)]
        json: bool,
        /// Use the native Rust engine instead of the Pulumi CLI.
        #[arg(long)]
        native: bool,
        /// Program binary to run (required with --native).
        #[arg(long)]
        program: Option<String>,
    },
    /// Refresh stack state from the cloud.
    Refresh {
        /// The stack to operate on.
        #[arg(short, long)]
        stack: String,
        /// Emit output as JSON.
        #[arg(long)]
        json: bool,
        /// Use the native Rust engine instead of the Pulumi CLI.
        #[arg(long)]
        native: bool,
        /// Program binary to run (required with --native).
        #[arg(long)]
        program: Option<String>,
    },
    /// Manage stacks.
    Stack {
        #[command(subcommand)]
        command: StackCommands,
    },
    /// Manage stack configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    /// Show the currently logged-in Pulumi user.
    Whoami {
        /// Emit output as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Fetch log entries for a stack.
    Logs {
        /// The stack to fetch logs for.
        #[arg(short, long)]
        stack: String,
    },
    /// Manage Pulumi plugins.
    Plugin {
        #[command(subcommand)]
        command: PluginCommands,
    },
}

#[derive(Subcommand)]
enum StackCommands {
    /// Initialize a new stack.
    Init {
        /// The name for the new stack.
        name: String,
    },
    /// List all stacks.
    Ls {
        /// Emit output as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Remove a stack.
    Rm {
        /// The stack to remove.
        name: String,
        /// Force removal even if the stack has resources.
        #[arg(long)]
        force: bool,
    },
    /// Show stack outputs.
    Output {
        /// The stack to query.
        #[arg(short, long)]
        stack: String,
    },
    /// Export the stack's deployment state as JSON.
    Export {
        /// The stack to export.
        #[arg(short, long)]
        stack: String,
    },
    /// Import a previously exported deployment state from stdin.
    Import {
        /// The stack to import into.
        #[arg(short, long)]
        stack: String,
        /// Path to the state file to import (reads from stdin if omitted).
        #[arg(short, long)]
        file: Option<String>,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Get a configuration value.
    Get {
        /// The config key.
        key: String,
        /// The stack to query.
        #[arg(short, long)]
        stack: String,
        /// Emit output as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Set a configuration value.
    Set {
        /// The config key.
        key: String,
        /// The config value.
        value: String,
        /// The stack to configure.
        #[arg(short, long)]
        stack: String,
        /// Mark the value as secret.
        #[arg(long)]
        secret: bool,
    },
    /// Remove a configuration value.
    Rm {
        /// The config key to remove.
        key: String,
        /// The stack to configure.
        #[arg(short, long)]
        stack: String,
    },
    /// List all configuration values.
    Ls {
        /// The stack to query.
        #[arg(short, long)]
        stack: String,
        /// Emit output as JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum PluginCommands {
    /// List installed plugins.
    Ls {
        /// Emit output as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Install a plugin.
    Install {
        /// Plugin kind (e.g. `resource`).
        kind: String,
        /// Plugin name (e.g. `aws`).
        name: String,
        /// Plugin version (e.g. `6.0.0`).
        version: String,
    },
    /// Remove a plugin.
    Rm {
        /// Plugin kind (e.g. `resource`).
        kind: String,
        /// Plugin name (e.g. `aws`).
        name: String,
        /// Plugin version to remove (removes all versions if omitted).
        version: Option<String>,
    },
}

/// Resolve the program path for native engine commands, exiting with an error if not provided.
#[cfg(feature = "native-engine")]
fn require_program(program: Option<String>) -> String {
    program.unwrap_or_else(|| {
        eprintln!("{} --program is required when using --native", "Error:".red().bold());
        std::process::exit(1);
    })
}

/// Format engine events as a human-readable diff for preview output.
fn format_diff_events(events: &[EngineEvent]) -> String {
    let mut lines = Vec::new();
    for ev in events {
        if let Some(rpe) = &ev.resource_pre_event {
            let md = &rpe.metadata;
            if md.op == "same" {
                continue;
            }
            let op_label = match md.op.as_str() {
                "create" => format!("{}", "+ create".green()),
                "delete" | "delete-replaced" => format!("{}", "- delete".red()),
                "update" | "replace" => format!("{}", "~ update".yellow()),
                other => format!("? {other}"),
            };
            let name = md.urn.rsplit("::").next().unwrap_or(md.urn.as_str());
            lines.push(format!(
                "  {op_label}  {}  {name}",
                md.resource_type.cyan()
            ));
        }
    }
    lines.join("\n")
}

/// Print an operation result, optionally in JSON, applying diff formatting for events.
fn print_result_stdout(stdout: &str, stderr: &str, json_mode: bool) {
    if json_mode {
        return; // caller handles JSON output
    }
    if !stdout.is_empty() {
        println!("{stdout}");
    }
    if !stderr.is_empty() {
        eprintln!("{stderr}");
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let ws = LocalWorkspace::new(&cli.cwd);

    let result: std::result::Result<(), pulumi_automation::error::Error> = match cli.command {
        Commands::Up { stack, json, native, program } => {
            // When --native is requested but feature not compiled, exit immediately.
            #[cfg(not(feature = "native-engine"))]
            let _ = &program; // not used without native-engine feature
            if native {
                #[cfg(not(feature = "native-engine"))]
                {
                    eprintln!(
                        "{} --native requires the native-engine feature (recompile with --features native-engine)",
                        "Error:".red().bold()
                    );
                    std::process::exit(1);
                }
            }

            let result = if native {
                #[cfg(feature = "native-engine")]
                {
                    let prog = require_program(program);
                    let ns = ws.native_stack(&stack, vec![prog]);
                    ns.up().await
                }
                #[cfg(not(feature = "native-engine"))]
                unreachable!()
            } else {
                let s = ws.select_stack(&stack).await.unwrap_or_else(|e| {
                    eprintln!("{} selecting stack: {e}", "Error:".red().bold());
                    std::process::exit(1);
                });
                s.up().await
            };

            match result {
                Ok(r) => {
                    if json {
                        let out = serde_json::json!({
                            "stdout": r.stdout,
                            "stderr": r.stderr,
                            "outputs": r.outputs,
                        });
                        println!("{}", serde_json::to_string_pretty(&out).unwrap());
                    } else {
                        print_result_stdout(&r.stdout, &r.stderr, false);
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Preview { stack, json, native, program } => {
            #[cfg(not(feature = "native-engine"))]
            let _ = &program;
            if native {
                #[cfg(not(feature = "native-engine"))]
                {
                    eprintln!(
                        "{} --native requires the native-engine feature",
                        "Error:".red().bold()
                    );
                    std::process::exit(1);
                }
            }

            let result = if native {
                #[cfg(feature = "native-engine")]
                {
                    let prog = require_program(program);
                    let ns = ws.native_stack(&stack, vec![prog]);
                    ns.preview().await
                }
                #[cfg(not(feature = "native-engine"))]
                unreachable!()
            } else {
                let s = ws.select_stack(&stack).await.unwrap_or_else(|e| {
                    eprintln!("{} selecting stack: {e}", "Error:".red().bold());
                    std::process::exit(1);
                });
                s.preview().await
            };

            match result {
                Ok(r) => {
                    if json {
                        let out = serde_json::json!({
                            "stdout": r.stdout,
                            "stderr": r.stderr,
                        });
                        println!("{}", serde_json::to_string_pretty(&out).unwrap());
                    } else {
                        print_result_stdout(&r.stdout, &r.stderr, false);
                        let diff = format_diff_events(&r.events);
                        if !diff.is_empty() {
                            println!("\nPending changes:\n{diff}");
                        }
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Destroy { stack, json, native, program } => {
            #[cfg(not(feature = "native-engine"))]
            let _ = &program;
            if native {
                #[cfg(not(feature = "native-engine"))]
                {
                    eprintln!(
                        "{} --native requires the native-engine feature",
                        "Error:".red().bold()
                    );
                    std::process::exit(1);
                }
            }

            let result = if native {
                #[cfg(feature = "native-engine")]
                {
                    let prog = require_program(program);
                    let ns = ws.native_stack(&stack, vec![prog]);
                    ns.destroy().await
                }
                #[cfg(not(feature = "native-engine"))]
                unreachable!()
            } else {
                let s = ws.select_stack(&stack).await.unwrap_or_else(|e| {
                    eprintln!("{} selecting stack: {e}", "Error:".red().bold());
                    std::process::exit(1);
                });
                s.destroy().await
            };

            match result {
                Ok(r) => {
                    if json {
                        let out = serde_json::json!({
                            "stdout": r.stdout,
                            "stderr": r.stderr,
                        });
                        println!("{}", serde_json::to_string_pretty(&out).unwrap());
                    } else {
                        print_result_stdout(&r.stdout, &r.stderr, false);
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Refresh { stack, json, native, program } => {
            #[cfg(not(feature = "native-engine"))]
            let _ = &program;
            if native {
                #[cfg(not(feature = "native-engine"))]
                {
                    eprintln!(
                        "{} --native requires the native-engine feature",
                        "Error:".red().bold()
                    );
                    std::process::exit(1);
                }
            }

            let result = if native {
                #[cfg(feature = "native-engine")]
                {
                    let prog = require_program(program);
                    let ns = ws.native_stack(&stack, vec![prog]);
                    ns.refresh().await
                }
                #[cfg(not(feature = "native-engine"))]
                unreachable!()
            } else {
                let s = ws.select_stack(&stack).await.unwrap_or_else(|e| {
                    eprintln!("{} selecting stack: {e}", "Error:".red().bold());
                    std::process::exit(1);
                });
                s.refresh().await
            };

            match result {
                Ok(r) => {
                    if json {
                        let out = serde_json::json!({
                            "stdout": r.stdout,
                            "stderr": r.stderr,
                        });
                        println!("{}", serde_json::to_string_pretty(&out).unwrap());
                    } else {
                        print_result_stdout(&r.stdout, &r.stderr, false);
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Stack { command } => match command {
            StackCommands::Init { name } => ws.create_stack(&name).await.map(|_| ()).map_err(|e| {
                eprintln!("{} creating stack: {e}", "Error:".red().bold());
                e
            }),
            StackCommands::Ls { json } => match ws.list_stacks().await {
                Ok(stacks) => {
                    if json {
                        println!("{}", serde_json::to_string_pretty(&stacks).unwrap());
                    } else {
                        for s in &stacks {
                            let current = if s.current { " *" } else { "" };
                            println!("{}{current}", s.name);
                        }
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            },
            StackCommands::Rm { name, force } => ws.remove_stack(&name, force).await,
            StackCommands::Output { stack } => {
                let s = ws.select_stack(&stack).await.unwrap_or_else(|e| {
                    eprintln!("{} selecting stack: {e}", "Error:".red().bold());
                    std::process::exit(1);
                });
                match s.outputs().await {
                    Ok(outputs) => {
                        let json =
                            serde_json::to_string_pretty(&outputs).expect("failed to serialize outputs");
                        println!("{json}");
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            StackCommands::Export { stack } => {
                let s = ws.select_stack(&stack).await.unwrap_or_else(|e| {
                    eprintln!("{} selecting stack: {e}", "Error:".red().bold());
                    std::process::exit(1);
                });
                match s.export_state().await {
                    Ok(state) => {
                        println!("{}", serde_json::to_string_pretty(&state).unwrap());
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            StackCommands::Import { stack, file } => {
                let s = ws.select_stack(&stack).await.unwrap_or_else(|e| {
                    eprintln!("{} selecting stack: {e}", "Error:".red().bold());
                    std::process::exit(1);
                });
                let content = match file {
                    Some(path) => std::fs::read_to_string(&path).unwrap_or_else(|e| {
                        eprintln!("{} reading file '{path}': {e}", "Error:".red().bold());
                        std::process::exit(1);
                    }),
                    None => {
                        use std::io::Read;
                        let mut buf = String::new();
                        std::io::stdin().read_to_string(&mut buf).unwrap_or_else(|e| {
                            eprintln!("{} reading stdin: {e}", "Error:".red().bold());
                            std::process::exit(1);
                        });
                        buf
                    }
                };
                let state: serde_json::Value =
                    serde_json::from_str(&content).unwrap_or_else(|e| {
                        eprintln!("{} parsing JSON: {e}", "Error:".red().bold());
                        std::process::exit(1);
                    });
                s.import_state(&state).await
            }
        },
        Commands::Config { command } => match command {
            ConfigCommands::Get { key, stack, json } => {
                match ws.get_config(&stack, &key).await {
                    Ok(cv) => {
                        if json {
                            println!("{}", serde_json::to_string_pretty(&cv).unwrap());
                        } else {
                            println!("{}", cv.value);
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            ConfigCommands::Set { key, value, stack, secret } => {
                let cv = if secret {
                    ConfigValue::secret(value)
                } else {
                    ConfigValue::plaintext(value)
                };
                ws.set_config(&stack, &key, &cv).await
            }
            ConfigCommands::Rm { key, stack } => ws.remove_config(&stack, &key).await,
            ConfigCommands::Ls { stack, json } => match ws.get_all_config(&stack).await {
                Ok(config) => {
                    if json {
                        println!("{}", serde_json::to_string_pretty(&config).unwrap());
                    } else {
                        for (key, cv) in &config {
                            let secret_marker = if cv.secret { " [secret]" } else { "" };
                            println!("{key}: {}{secret_marker}", cv.value);
                        }
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            },
        },
        Commands::Whoami { json } => match ws.whoami().await {
            Ok(result) => {
                if json {
                    println!("{}", serde_json::to_string_pretty(&result).unwrap());
                } else {
                    println!("user: {}", result.user);
                    if let Some(url) = &result.url {
                        println!("url:  {url}");
                    }
                    if !result.organizations.is_empty() {
                        println!("orgs: {}", result.organizations.join(", "));
                    }
                }
                Ok(())
            }
            Err(e) => Err(e),
        },
        Commands::Logs { stack } => match ws.logs(&stack).await {
            Ok(output) => {
                print!("{output}");
                Ok(())
            }
            Err(e) => Err(e),
        },
        Commands::Plugin { command } => match command {
            PluginCommands::Ls { json } => match ws.list_plugins().await {
                Ok(plugins) => {
                    if json {
                        println!("{}", serde_json::to_string_pretty(&plugins).unwrap());
                    } else {
                        for p in &plugins {
                            println!("{} {} v{}", p.kind, p.name, p.version);
                        }
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            },
            PluginCommands::Install { kind, name, version } => {
                ws.install_plugin(&kind, &name, &version).await
            }
            PluginCommands::Rm { kind, name, version } => {
                ws.remove_plugin(&kind, &name, version.as_deref()).await
            }
        },
    };

    if let Err(e) = result {
        eprintln!("{} {e}", "Error:".red().bold());
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn test_up_command() {
        let cli = Cli::parse_from(["pulumi-rs", "up", "--stack", "dev"]);
        assert!(
            matches!(cli.command, Commands::Up { stack, json: false, native: false, program: None } if stack == "dev")
        );
    }

    #[test]
    fn test_up_command_json() {
        let cli = Cli::parse_from(["pulumi-rs", "up", "--stack", "dev", "--json"]);
        assert!(
            matches!(cli.command, Commands::Up { stack, json: true, native: false, .. } if stack == "dev")
        );
    }

    #[test]
    fn test_up_native_flag() {
        let cli = Cli::parse_from([
            "pulumi-rs", "up", "--stack", "dev", "--native", "--program", "./my-prog",
        ]);
        assert!(matches!(
            cli.command,
            Commands::Up { stack, native: true, program: Some(p), .. }
            if stack == "dev" && p == "./my-prog"
        ));
    }

    #[test]
    fn test_preview_command() {
        let cli = Cli::parse_from(["pulumi-rs", "preview", "--stack", "staging"]);
        assert!(
            matches!(cli.command, Commands::Preview { stack, json: false, native: false, .. } if stack == "staging")
        );
    }

    #[test]
    fn test_preview_native_flag() {
        let cli = Cli::parse_from([
            "pulumi-rs", "preview", "--stack", "dev", "--native", "--program", "./bin",
        ]);
        assert!(matches!(
            cli.command,
            Commands::Preview { native: true, program: Some(_), .. }
        ));
    }

    #[test]
    fn test_destroy_command() {
        let cli = Cli::parse_from(["pulumi-rs", "destroy", "--stack", "dev"]);
        assert!(
            matches!(cli.command, Commands::Destroy { stack, json: false, native: false, .. } if stack == "dev")
        );
    }

    #[test]
    fn test_refresh_command() {
        let cli = Cli::parse_from(["pulumi-rs", "refresh", "--stack", "dev"]);
        assert!(
            matches!(cli.command, Commands::Refresh { stack, json: false, native: false, .. } if stack == "dev")
        );
    }

    #[test]
    fn test_stack_init() {
        let cli = Cli::parse_from(["pulumi-rs", "stack", "init", "prod"]);
        assert!(matches!(
            cli.command,
            Commands::Stack { command: StackCommands::Init { name } } if name == "prod"
        ));
    }

    #[test]
    fn test_stack_ls() {
        let cli = Cli::parse_from(["pulumi-rs", "stack", "ls"]);
        assert!(matches!(
            cli.command,
            Commands::Stack { command: StackCommands::Ls { json: false } }
        ));
    }

    #[test]
    fn test_stack_ls_json() {
        let cli = Cli::parse_from(["pulumi-rs", "stack", "ls", "--json"]);
        assert!(matches!(
            cli.command,
            Commands::Stack { command: StackCommands::Ls { json: true } }
        ));
    }

    #[test]
    fn test_stack_rm() {
        let cli = Cli::parse_from(["pulumi-rs", "stack", "rm", "dev"]);
        assert!(matches!(
            cli.command,
            Commands::Stack { command: StackCommands::Rm { name, force: false } } if name == "dev"
        ));
    }

    #[test]
    fn test_stack_rm_force() {
        let cli = Cli::parse_from(["pulumi-rs", "stack", "rm", "dev", "--force"]);
        assert!(matches!(
            cli.command,
            Commands::Stack { command: StackCommands::Rm { name, force: true } } if name == "dev"
        ));
    }

    #[test]
    fn test_stack_output() {
        let cli = Cli::parse_from(["pulumi-rs", "stack", "output", "--stack", "dev"]);
        assert!(matches!(
            cli.command,
            Commands::Stack { command: StackCommands::Output { stack } } if stack == "dev"
        ));
    }

    #[test]
    fn test_stack_export() {
        let cli = Cli::parse_from(["pulumi-rs", "stack", "export", "--stack", "dev"]);
        assert!(matches!(
            cli.command,
            Commands::Stack { command: StackCommands::Export { stack } } if stack == "dev"
        ));
    }

    #[test]
    fn test_stack_import() {
        let cli = Cli::parse_from(["pulumi-rs", "stack", "import", "--stack", "dev"]);
        assert!(matches!(
            cli.command,
            Commands::Stack { command: StackCommands::Import { stack, file: None } } if stack == "dev"
        ));
    }

    #[test]
    fn test_stack_import_with_file() {
        let cli = Cli::parse_from([
            "pulumi-rs", "stack", "import", "--stack", "dev", "--file", "state.json",
        ]);
        assert!(matches!(
            cli.command,
            Commands::Stack { command: StackCommands::Import { stack, file: Some(f) } }
            if stack == "dev" && f == "state.json"
        ));
    }

    #[test]
    fn test_config_get() {
        let cli =
            Cli::parse_from(["pulumi-rs", "config", "get", "aws:region", "--stack", "dev"]);
        assert!(matches!(
            cli.command,
            Commands::Config { command: ConfigCommands::Get { key, stack, json: false } }
            if key == "aws:region" && stack == "dev"
        ));
    }

    #[test]
    fn test_config_get_json() {
        let cli = Cli::parse_from([
            "pulumi-rs", "config", "get", "aws:region", "--stack", "dev", "--json",
        ]);
        assert!(matches!(
            cli.command,
            Commands::Config { command: ConfigCommands::Get { key, stack, json: true } }
            if key == "aws:region" && stack == "dev"
        ));
    }

    #[test]
    fn test_config_set() {
        let cli = Cli::parse_from([
            "pulumi-rs", "config", "set", "aws:region", "us-east-1", "--stack", "dev",
        ]);
        assert!(matches!(
            cli.command,
            Commands::Config {
                command: ConfigCommands::Set { key, value, stack, secret: false }
            } if key == "aws:region" && value == "us-east-1" && stack == "dev"
        ));
    }

    #[test]
    fn test_config_set_secret() {
        let cli = Cli::parse_from([
            "pulumi-rs", "config", "set", "db:password", "hunter2", "--stack", "dev", "--secret",
        ]);
        assert!(matches!(
            cli.command,
            Commands::Config {
                command: ConfigCommands::Set { key, value, stack, secret: true }
            } if key == "db:password" && value == "hunter2" && stack == "dev"
        ));
    }

    #[test]
    fn test_config_rm() {
        let cli =
            Cli::parse_from(["pulumi-rs", "config", "rm", "aws:region", "--stack", "dev"]);
        assert!(matches!(
            cli.command,
            Commands::Config { command: ConfigCommands::Rm { key, stack } }
            if key == "aws:region" && stack == "dev"
        ));
    }

    #[test]
    fn test_config_ls() {
        let cli = Cli::parse_from(["pulumi-rs", "config", "ls", "--stack", "dev"]);
        assert!(matches!(
            cli.command,
            Commands::Config { command: ConfigCommands::Ls { stack, json: false } }
            if stack == "dev"
        ));
    }

    #[test]
    fn test_config_ls_json() {
        let cli = Cli::parse_from(["pulumi-rs", "config", "ls", "--stack", "dev", "--json"]);
        assert!(matches!(
            cli.command,
            Commands::Config { command: ConfigCommands::Ls { stack, json: true } }
            if stack == "dev"
        ));
    }

    #[test]
    fn test_whoami_command() {
        let cli = Cli::parse_from(["pulumi-rs", "whoami"]);
        assert!(matches!(cli.command, Commands::Whoami { json: false }));
    }

    #[test]
    fn test_whoami_json() {
        let cli = Cli::parse_from(["pulumi-rs", "whoami", "--json"]);
        assert!(matches!(cli.command, Commands::Whoami { json: true }));
    }

    #[test]
    fn test_logs_command() {
        let cli = Cli::parse_from(["pulumi-rs", "logs", "--stack", "dev"]);
        assert!(matches!(cli.command, Commands::Logs { stack } if stack == "dev"));
    }

    #[test]
    fn test_plugin_ls() {
        let cli = Cli::parse_from(["pulumi-rs", "plugin", "ls"]);
        assert!(matches!(
            cli.command,
            Commands::Plugin { command: PluginCommands::Ls { json: false } }
        ));
    }

    #[test]
    fn test_plugin_ls_json() {
        let cli = Cli::parse_from(["pulumi-rs", "plugin", "ls", "--json"]);
        assert!(matches!(
            cli.command,
            Commands::Plugin { command: PluginCommands::Ls { json: true } }
        ));
    }

    #[test]
    fn test_plugin_install() {
        let cli =
            Cli::parse_from(["pulumi-rs", "plugin", "install", "resource", "aws", "6.0.0"]);
        assert!(matches!(
            cli.command,
            Commands::Plugin {
                command: PluginCommands::Install { kind, name, version }
            } if kind == "resource" && name == "aws" && version == "6.0.0"
        ));
    }

    #[test]
    fn test_plugin_rm() {
        let cli = Cli::parse_from(["pulumi-rs", "plugin", "rm", "resource", "aws"]);
        assert!(matches!(
            cli.command,
            Commands::Plugin {
                command: PluginCommands::Rm { kind, name, version: None }
            } if kind == "resource" && name == "aws"
        ));
    }

    #[test]
    fn test_plugin_rm_with_version() {
        let cli =
            Cli::parse_from(["pulumi-rs", "plugin", "rm", "resource", "aws", "6.0.0"]);
        assert!(matches!(
            cli.command,
            Commands::Plugin {
                command: PluginCommands::Rm { kind, name, version: Some(v) }
            } if kind == "resource" && name == "aws" && v == "6.0.0"
        ));
    }

    #[test]
    fn test_cwd_default() {
        let cli = Cli::parse_from(["pulumi-rs", "stack", "ls"]);
        assert_eq!(cli.cwd, ".");
    }

    #[test]
    fn test_cwd_custom() {
        let cli = Cli::parse_from(["pulumi-rs", "--cwd", "/tmp/project", "stack", "ls"]);
        assert_eq!(cli.cwd, "/tmp/project");
    }

    #[test]
    fn test_format_diff_events_empty() {
        assert_eq!(format_diff_events(&[]), "");
    }

    #[test]
    fn test_format_diff_events_skips_same() {
        use pulumi_automation::event::{ResourcePreEvent, StepEventMetadata};
        let events = vec![EngineEvent {
            sequence: 1,
            prelude_event: None,
            resource_pre_event: Some(ResourcePreEvent {
                metadata: StepEventMetadata {
                    op: "same".into(),
                    urn: "urn:pulumi:dev::proj::aws:s3/bucket:Bucket::my-bucket".into(),
                    resource_type: "aws:s3/bucket:Bucket".into(),
                    old: None,
                    new: None,
                },
            }),
            summary_event: None,
            diagnostic_event: None,
        }];
        assert_eq!(format_diff_events(&events), "");
    }

    #[test]
    fn test_format_diff_events_create() {
        use pulumi_automation::event::{ResourcePreEvent, StepEventMetadata};
        let events = vec![EngineEvent {
            sequence: 1,
            prelude_event: None,
            resource_pre_event: Some(ResourcePreEvent {
                metadata: StepEventMetadata {
                    op: "create".into(),
                    urn: "urn:pulumi:dev::proj::aws:s3/bucket:Bucket::my-bucket".into(),
                    resource_type: "aws:s3/bucket:Bucket".into(),
                    old: None,
                    new: None,
                },
            }),
            summary_event: None,
            diagnostic_event: None,
        }];
        let diff = format_diff_events(&events);
        assert!(diff.contains("my-bucket"));
        assert!(diff.contains("aws:s3/bucket:Bucket"));
    }
}
