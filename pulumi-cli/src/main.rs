//! # pulumi-rs CLI
//!
//! A Rust-native CLI for Pulumi stack operations, built on `pulumi-automation`.

use clap::{Parser, Subcommand};
use pulumi_automation::config::ConfigValue;
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
    },
    /// Show a preview of pending changes.
    Preview {
        /// The stack to operate on.
        #[arg(short, long)]
        stack: String,
        /// Emit output as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Destroy all resources in a stack.
    Destroy {
        /// The stack to operate on.
        #[arg(short, long)]
        stack: String,
        /// Emit output as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Refresh stack state from the cloud.
    Refresh {
        /// The stack to operate on.
        #[arg(short, long)]
        stack: String,
        /// Emit output as JSON.
        #[arg(long)]
        json: bool,
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

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let ws = LocalWorkspace::new(&cli.cwd);

    let result: std::result::Result<(), pulumi_automation::error::Error> = match cli.command {
        Commands::Up { stack, json } => {
            let s = ws.select_stack(&stack).await.unwrap_or_else(|e| {
                eprintln!("Error selecting stack: {e}");
                std::process::exit(1);
            });
            match s.up().await {
                Ok(r) => {
                    if json {
                        let out = serde_json::json!({
                            "stdout": r.stdout,
                            "stderr": r.stderr,
                            "outputs": r.outputs,
                        });
                        println!("{}", serde_json::to_string_pretty(&out).unwrap());
                    } else {
                        println!("{}", r.stdout);
                        if !r.stderr.is_empty() {
                            eprintln!("{}", r.stderr);
                        }
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Preview { stack, json } => {
            let s = ws.select_stack(&stack).await.unwrap_or_else(|e| {
                eprintln!("Error selecting stack: {e}");
                std::process::exit(1);
            });
            match s.preview().await {
                Ok(r) => {
                    if json {
                        let out = serde_json::json!({
                            "stdout": r.stdout,
                            "stderr": r.stderr,
                        });
                        println!("{}", serde_json::to_string_pretty(&out).unwrap());
                    } else {
                        println!("{}", r.stdout);
                        if !r.stderr.is_empty() {
                            eprintln!("{}", r.stderr);
                        }
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Destroy { stack, json } => {
            let s = ws.select_stack(&stack).await.unwrap_or_else(|e| {
                eprintln!("Error selecting stack: {e}");
                std::process::exit(1);
            });
            match s.destroy().await {
                Ok(r) => {
                    if json {
                        let out = serde_json::json!({
                            "stdout": r.stdout,
                            "stderr": r.stderr,
                        });
                        println!("{}", serde_json::to_string_pretty(&out).unwrap());
                    } else {
                        println!("{}", r.stdout);
                        if !r.stderr.is_empty() {
                            eprintln!("{}", r.stderr);
                        }
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Refresh { stack, json } => {
            let s = ws.select_stack(&stack).await.unwrap_or_else(|e| {
                eprintln!("Error selecting stack: {e}");
                std::process::exit(1);
            });
            match s.refresh().await {
                Ok(r) => {
                    if json {
                        let out = serde_json::json!({
                            "stdout": r.stdout,
                            "stderr": r.stderr,
                        });
                        println!("{}", serde_json::to_string_pretty(&out).unwrap());
                    } else {
                        println!("{}", r.stdout);
                        if !r.stderr.is_empty() {
                            eprintln!("{}", r.stderr);
                        }
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Stack { command } => match command {
            StackCommands::Init { name } => ws.create_stack(&name).await.map(|_| ()).map_err(|e| {
                eprintln!("Error creating stack: {e}");
                e
            }),
            StackCommands::Ls { json } => match ws.list_stacks().await {
                Ok(stacks) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&stacks).unwrap()
                        );
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
                    eprintln!("Error selecting stack: {e}");
                    std::process::exit(1);
                });
                match s.outputs().await {
                    Ok(outputs) => {
                        let json = serde_json::to_string_pretty(&outputs)
                            .expect("failed to serialize outputs");
                        println!("{json}");
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            StackCommands::Export { stack } => {
                let s = ws.select_stack(&stack).await.unwrap_or_else(|e| {
                    eprintln!("Error selecting stack: {e}");
                    std::process::exit(1);
                });
                match s.export_state().await {
                    Ok(state) => {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&state).unwrap()
                        );
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            StackCommands::Import { stack, file } => {
                let s = ws.select_stack(&stack).await.unwrap_or_else(|e| {
                    eprintln!("Error selecting stack: {e}");
                    std::process::exit(1);
                });
                let content = match file {
                    Some(path) => std::fs::read_to_string(&path).unwrap_or_else(|e| {
                        eprintln!("Error reading file '{path}': {e}");
                        std::process::exit(1);
                    }),
                    None => {
                        use std::io::Read;
                        let mut buf = String::new();
                        std::io::stdin().read_to_string(&mut buf).unwrap_or_else(|e| {
                            eprintln!("Error reading stdin: {e}");
                            std::process::exit(1);
                        });
                        buf
                    }
                };
                let state: serde_json::Value =
                    serde_json::from_str(&content).unwrap_or_else(|e| {
                        eprintln!("Error parsing JSON: {e}");
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
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&cv).unwrap()
                            );
                        } else {
                            println!("{}", cv.value);
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            ConfigCommands::Set {
                key,
                value,
                stack,
                secret,
            } => {
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
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&config).unwrap()
                        );
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
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
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
        assert!(matches!(cli.command, Commands::Up { stack, json: false } if stack == "dev"));
    }

    #[test]
    fn test_up_command_json() {
        let cli = Cli::parse_from(["pulumi-rs", "up", "--stack", "dev", "--json"]);
        assert!(matches!(cli.command, Commands::Up { stack, json: true } if stack == "dev"));
    }

    #[test]
    fn test_preview_command() {
        let cli = Cli::parse_from(["pulumi-rs", "preview", "--stack", "staging"]);
        assert!(
            matches!(cli.command, Commands::Preview { stack, json: false } if stack == "staging")
        );
    }

    #[test]
    fn test_destroy_command() {
        let cli = Cli::parse_from(["pulumi-rs", "destroy", "--stack", "dev"]);
        assert!(
            matches!(cli.command, Commands::Destroy { stack, json: false } if stack == "dev")
        );
    }

    #[test]
    fn test_refresh_command() {
        let cli = Cli::parse_from(["pulumi-rs", "refresh", "--stack", "dev"]);
        assert!(
            matches!(cli.command, Commands::Refresh { stack, json: false } if stack == "dev")
        );
    }

    #[test]
    fn test_stack_init() {
        let cli = Cli::parse_from(["pulumi-rs", "stack", "init", "prod"]);
        assert!(matches!(
            cli.command,
            Commands::Stack {
                command: StackCommands::Init { name }
            } if name == "prod"
        ));
    }

    #[test]
    fn test_stack_ls() {
        let cli = Cli::parse_from(["pulumi-rs", "stack", "ls"]);
        assert!(matches!(
            cli.command,
            Commands::Stack {
                command: StackCommands::Ls { json: false }
            }
        ));
    }

    #[test]
    fn test_stack_ls_json() {
        let cli = Cli::parse_from(["pulumi-rs", "stack", "ls", "--json"]);
        assert!(matches!(
            cli.command,
            Commands::Stack {
                command: StackCommands::Ls { json: true }
            }
        ));
    }

    #[test]
    fn test_stack_rm() {
        let cli = Cli::parse_from(["pulumi-rs", "stack", "rm", "dev"]);
        assert!(matches!(
            cli.command,
            Commands::Stack {
                command: StackCommands::Rm { name, force: false }
            } if name == "dev"
        ));
    }

    #[test]
    fn test_stack_rm_force() {
        let cli = Cli::parse_from(["pulumi-rs", "stack", "rm", "dev", "--force"]);
        assert!(matches!(
            cli.command,
            Commands::Stack {
                command: StackCommands::Rm { name, force: true }
            } if name == "dev"
        ));
    }

    #[test]
    fn test_stack_output() {
        let cli = Cli::parse_from(["pulumi-rs", "stack", "output", "--stack", "dev"]);
        assert!(matches!(
            cli.command,
            Commands::Stack {
                command: StackCommands::Output { stack }
            } if stack == "dev"
        ));
    }

    #[test]
    fn test_stack_export() {
        let cli = Cli::parse_from(["pulumi-rs", "stack", "export", "--stack", "dev"]);
        assert!(matches!(
            cli.command,
            Commands::Stack {
                command: StackCommands::Export { stack }
            } if stack == "dev"
        ));
    }

    #[test]
    fn test_stack_import() {
        let cli = Cli::parse_from(["pulumi-rs", "stack", "import", "--stack", "dev"]);
        assert!(matches!(
            cli.command,
            Commands::Stack {
                command: StackCommands::Import { stack, file: None }
            } if stack == "dev"
        ));
    }

    #[test]
    fn test_stack_import_with_file() {
        let cli = Cli::parse_from([
            "pulumi-rs",
            "stack",
            "import",
            "--stack",
            "dev",
            "--file",
            "state.json",
        ]);
        assert!(matches!(
            cli.command,
            Commands::Stack {
                command: StackCommands::Import { stack, file: Some(f) }
            } if stack == "dev" && f == "state.json"
        ));
    }

    #[test]
    fn test_config_get() {
        let cli = Cli::parse_from(["pulumi-rs", "config", "get", "aws:region", "--stack", "dev"]);
        assert!(matches!(
            cli.command,
            Commands::Config {
                command: ConfigCommands::Get { key, stack, json: false }
            } if key == "aws:region" && stack == "dev"
        ));
    }

    #[test]
    fn test_config_get_json() {
        let cli = Cli::parse_from([
            "pulumi-rs", "config", "get", "aws:region", "--stack", "dev", "--json",
        ]);
        assert!(matches!(
            cli.command,
            Commands::Config {
                command: ConfigCommands::Get { key, stack, json: true }
            } if key == "aws:region" && stack == "dev"
        ));
    }

    #[test]
    fn test_config_set() {
        let cli = Cli::parse_from([
            "pulumi-rs",
            "config",
            "set",
            "aws:region",
            "us-east-1",
            "--stack",
            "dev",
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
            "pulumi-rs",
            "config",
            "set",
            "db:password",
            "hunter2",
            "--stack",
            "dev",
            "--secret",
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
        let cli = Cli::parse_from([
            "pulumi-rs", "config", "rm", "aws:region", "--stack", "dev",
        ]);
        assert!(matches!(
            cli.command,
            Commands::Config {
                command: ConfigCommands::Rm { key, stack }
            } if key == "aws:region" && stack == "dev"
        ));
    }

    #[test]
    fn test_config_ls() {
        let cli = Cli::parse_from(["pulumi-rs", "config", "ls", "--stack", "dev"]);
        assert!(matches!(
            cli.command,
            Commands::Config {
                command: ConfigCommands::Ls { stack, json: false }
            } if stack == "dev"
        ));
    }

    #[test]
    fn test_config_ls_json() {
        let cli = Cli::parse_from(["pulumi-rs", "config", "ls", "--stack", "dev", "--json"]);
        assert!(matches!(
            cli.command,
            Commands::Config {
                command: ConfigCommands::Ls { stack, json: true }
            } if stack == "dev"
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
}
