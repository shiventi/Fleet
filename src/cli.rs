use clap::Args;
use clap::Parser;
use clap::Subcommand;
use std::path::PathBuf;

/// The command selected by the user.
#[derive(Parser, Debug)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

/// Operations supported by Fleet's command-line interface.
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run the application described by a manifest file.
    Run {
        /// Path to the application manifest.
        manifest: PathBuf,
    },
    /// Check a manifest without starting any processes.
    Validate {
        /// Path to the application manifest.
        manifest: PathBuf,
    },

    /// Run a workload and answer status requests.
    Agent {
        #[arg(long)]
        listen: String,
        #[arg(long)]
        manifest: PathBuf,
    },

    Status(StatusArgs),
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    #[arg(short, long)]
    pub address: String,
}
