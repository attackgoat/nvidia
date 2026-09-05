#![forbid(unsafe_code)]

use {
    anyhow::Result,
    clap::{Parser, Subcommand},
    nvidia_sdk::{ResolveOptions, RuntimeProfile, Sdk},
    std::path::PathBuf,
};

#[derive(Parser)]
#[command(about = "Acquire and stage pinned NVIDIA SDK files")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Resolve an SDK and print its validated path.
    Path {
        sdk: Sdk,
        #[arg(long)]
        target: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        offline: bool,
    },
    /// Resolve an SDK and stage its redistributable runtime files.
    Stage {
        sdk: Sdk,
        #[arg(long)]
        target: String,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value = "release")]
        profile: RuntimeProfile,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        offline: bool,
    },
    /// Resolve and validate an SDK without staging files.
    Verify {
        sdk: Sdk,
        #[arg(long)]
        target: String,
        #[arg(long)]
        offline: bool,
    },
}

impl Command {
    fn run(self) -> Result<()> {
        match self {
            Command::Path {
                sdk,
                target,
                json,
                offline,
            } => {
                let resolved = sdk.resolve(&Self::options(target, offline))?;

                if json {
                    println!("{}", serde_json::to_string_pretty(&resolved)?);
                } else {
                    println!("{}", resolved.path.display());
                }
            }
            Command::Stage {
                sdk,
                target,
                output,
                profile,
                json,
                offline,
            } => {
                let resolved = sdk.resolve(&Self::options(target, offline))?;
                let files = resolved.stage_runtime(&output, profile)?;

                if json {
                    println!("{}", serde_json::to_string_pretty(&files)?);
                } else {
                    for file in files {
                        println!("{}", file.path.display());
                    }
                }
            }
            Command::Verify {
                sdk,
                target,
                offline,
            } => {
                let resolved = sdk.resolve(&Self::options(target, offline))?;

                println!(
                    "verified {} {} for {} at {}",
                    resolved.sdk.name(),
                    resolved.version,
                    resolved.target,
                    resolved.path.display()
                );
            }
        }

        Ok(())
    }

    fn options(target: String, offline: bool) -> ResolveOptions {
        let mut options = ResolveOptions::for_target(target);
        options.offline |= offline;

        options
    }
}

fn main() -> Result<()> {
    Args::parse().command.run()
}
