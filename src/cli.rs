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

    /// Create or reuse this device's identity and print its public key or fingerprint.
    Identity {
        /// Print the public key's SHA-256 fingerprint instead of its PEM text.
        #[arg(long)]
        fingerprint: bool,
    },

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

    /// List, inspect, add, or remove approved device keys.
    Peers {
        #[command(subcommand)]
        command: PeerCommands,
    },
}

/// Operations on the local list of approved device keys.
#[derive(Subcommand, Debug)]
pub enum PeerCommands {
    /// Show approved device names.
    List,
    /// Add a device using its exported public key, never its private key.
    Add {
        /// A unique label using ASCII letters, digits, hyphens, underscores, or dots.
        name: String,
        /// A public-key PEM file exported by the other device's identity command.
        public_key_file: PathBuf,
    },

    /// Remove a device from the saved list.
    Remove {
        /// The saved device label to remove; this does not revoke TLS certificates.
        name: String,
    },
    /// Show one saved device's name, fingerprint, and public key.
    Show {
        /// The saved device label to inspect.
        name: String,
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
