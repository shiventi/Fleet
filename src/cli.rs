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

    /// Create or reuse this device's identity and print only its public key.
    Identity,

    /// Run a workload and accept commands over mutual TLS.
    Agent {
        #[arg(long)]
        listen: String,
        #[arg(long)]
        manifest: PathBuf,
        #[command(flatten)]
        tls: TlsFiles,
        /// Allow binding to a specific remote IP; use a private network.
        #[arg(long)]
        allow_remote: bool,
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

    /// Keep checking the agents listed in a cluster file.
    Watch {
        #[arg(long)]
        manifest: PathBuf,
        #[command(flatten)]
        tls: TlsFiles,
    },

    /// Show live agent counts and scale the selected node with = or -.
    Dashboard {
        /// Path to the cluster manifest.
        #[arg(long)]
        manifest: PathBuf,
        /// Trusted CA and client identity used to check the agents.
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
