//! # pulumi-rs CLI
//!
//! A Rust-native CLI for Pulumi stack operations, built on `pulumi-automation`.

use clap::{Parser, Subcommand};
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
    },
    /// Show a preview of pending changes.
    Preview {
        /// The stack to operate on.
        #[arg(short, long)]
        stack: String,
    },
    /// Destroy all resources in a stack.
    Destroy {
        /// The stack to operate on.
        #[arg(short, long)]
        stack: String,
    },
    /// Refresh stack state from the cloud.
    Refresh {
        /// The stack to operate on.
        #[arg(short, long)]
        stack: String,
    },
    /// Manage stacks.
    Stack {
        #[command(subcommand)]
        command: StackCommands,
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
    Ls,
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
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let ws = LocalWorkspace::new(&cli.cwd);

    let result = match cli.command {
        Commands::Up { stack } => {
            let s = ws.select_stack(&stack).await.unwrap_or_else(|e| {
                eprintln!("Error selecting stack: {e}");
                std::process::exit(1);
            });
            match s.up().await {
                Ok(r) => {
                    println!("{}", r.stdout);
                    if !r.stderr.is_empty() {
                        eprintln!("{}", r.stderr);
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Preview { stack } => {
            let s = ws.select_stack(&stack).await.unwrap_or_else(|e| {
                eprintln!("Error selecting stack: {e}");
                std::process::exit(1);
            });
            match s.preview().await {
                Ok(r) => {
                    println!("{}", r.stdout);
                    if !r.stderr.is_empty() {
                        eprintln!("{}", r.stderr);
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Destroy { stack } => {
            let s = ws.select_stack(&stack).await.unwrap_or_else(|e| {
                eprintln!("Error selecting stack: {e}");
                std::process::exit(1);
            });
            match s.destroy().await {
                Ok(r) => {
                    println!("{}", r.stdout);
                    if !r.stderr.is_empty() {
                        eprintln!("{}", r.stderr);
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        Commands::Refresh { stack } => {
            let s = ws.select_stack(&stack).await.unwrap_or_else(|e| {
                eprintln!("Error selecting stack: {e}");
                std::process::exit(1);
            });
            match s.refresh().await {
                Ok(r) => {
                    println!("{}", r.stdout);
                    if !r.stderr.is_empty() {
                        eprintln!("{}", r.stderr);
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
            StackCommands::Ls => match ws.list_stacks().await {
                Ok(stacks) => {
                    for s in &stacks {
                        let current = if s.current { " *" } else { "" };
                        println!("{}{current}", s.name);
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
        },
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
