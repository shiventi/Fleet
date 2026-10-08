use crate::tls::TlsFiles;
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

    /// Run a workload and accept commands over mutual TLS.
    Agent {
        #[arg(long)]
        listen: String,
        #[arg(long)]
        manifest: PathBuf,
        #[command(flatten)]
        tls: TlsFiles,
    },

    /// Show a workload's current counts over mutual TLS.
    Status(StatusArgs),

    /// Stop the workload while keeping the agent running.
    Stop {
        #[arg(long)]
        address: String,
        /// Expected name in the agent's certificate, not its connection address.
        #[arg(long, default_value = "localhost")]
        server_name: String,
        #[command(flatten)]
        tls: TlsFiles,
    },

    /// Change the number of copies managed by the agent.
    Scale {
        #[arg(long)]
        replicas: u32,
        #[arg(long)]
        address: String,
        /// Expected name in the agent's certificate, not its connection address.
        #[arg(long, default_value = "localhost")]
        server_name: String,
        #[command(flatten)]
        tls: TlsFiles,
    },
}

/// Connection address and trusted TLS identity for a status request.
#[derive(Args, Debug)]
pub struct StatusArgs {
    #[arg(short, long)]
    pub address: String,
    /// Expected name in the agent's certificate, not its connection address.
    #[arg(long, default_value = "localhost")]
    pub server_name: String,
    #[command(flatten)]
    pub tls: TlsFiles,
}
