use crate::tls::TlsFiles;
use clap::Args;
use clap::Parser;
use clap::Subcommand;
use std::path::PathBuf;

// names and flags for the cli
#[derive(Parser, Debug)]
#[command(about = "run and check apps on your servers")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    #[command(about = "run an app from a file")]
    Run {
        #[arg(help = "app file")]
        manifest: PathBuf,
    },
    #[command(about = "check an app file without running it")]
    Validate {
        #[arg(help = "app file")]
        manifest: PathBuf,
    },

    #[command(about = "show this device's public key")]
    Identity {
        #[arg(help = "show the key hash instead")]
        #[arg(long)]
        fingerprint: bool,
    },

    #[command(about = "run an app and take remote commands")]
    Agent {
        #[arg(long)]
        listen: String,
        #[arg(long)]
        manifest: PathBuf,
        #[command(flatten)]
        tls: TlsFiles,
        #[arg(help = "allow a remote IP; use a private network")]
        #[arg(long)]
        allow_remote: bool,
    },

    #[command(about = "show an app's status")]
    Status(StatusArgs),

    #[command(about = "show an app's logs")]
    Logs {
        #[arg(long)]
        address: String,
        #[arg(long)]
        app: String,
        #[arg(long, default_value = "localhost")]
        server_name: String,
        #[command(flatten)]
        tls: TlsFiles,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(short, long)]
        follow: bool,
    },

    #[command(about = "show app logs with a saved device key")]
    LogsDevice {
        #[arg(long)]
        address: String,
        #[arg(long)]
        app: String,
        #[arg(long)]
        peer: String,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(short, long)]
        follow: bool,
    },

    #[command(about = "stop the app, not the agent")]
    Stop {
        #[arg(long)]
        address: String,
        #[arg(help = "name to check on the server cert")]
        #[arg(long, default_value = "localhost")]
        server_name: String,
        #[command(flatten)]
        tls: TlsFiles,
    },

    #[command(about = "set the app's copy count")]
    Scale {
        #[arg(long)]
        replicas: u32,
        #[arg(long)]
        address: String,
        #[arg(help = "name to check on the server cert")]
        #[arg(long, default_value = "localhost")]
        server_name: String,
        #[command(flatten)]
        tls: TlsFiles,
    },

    #[command(about = "watch the nodes in a file")]
    Watch {
        #[arg(long)]
        manifest: PathBuf,
        #[command(flatten)]
        tls: TlsFiles,
    },

    #[command(about = "watch nodes with saved keys")]
    WatchDevice {
        #[arg(long)]
        manifest: PathBuf,
    },

    #[command(about = "view nodes; use = or - to scale")]
    Dashboard {
        #[arg(help = "node list file")]
        #[arg(long)]
        manifest: PathBuf,
        #[command(flatten)]
        tls: TlsFiles,
    },

    #[command(about = "view and scale nodes with saved keys")]
    DashboardDevice {
        #[arg(long)]
        manifest: PathBuf,
    },

    #[command(about = "list, show, add, or drop device keys")]
    Peers {
        #[command(subcommand)]
        command: PeerCommands,
    },

    #[command(about = "run an agent with device keys")]
    AgentDevice {
        #[arg(long)]
        listen: String,
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        allow_remote: bool,
    },

    #[command(about = "check an app with its device key")]
    StatusDevice {
        #[arg(long)]
        address: String,
        #[arg(help = "saved name for this server")]
        #[arg(long)]
        peer: String,
    },

    #[command(about = "stop an app with its device key")]
    StopDevice {
        #[arg(long)]
        address: String,
        #[arg(long)]
        peer: String,
    },

    #[command(about = "set app copies with its device key")]
    ScaleDevice {
        #[arg(long)]
        address: String,
        #[arg(long)]
        peer: String,
        #[arg(long)]
        replicas: u32,
    },

    #[command(about = "show live cpu and ram usage")]
    Metrics,

}

#[derive(Subcommand, Debug)]
pub enum PeerCommands {
    #[command(about = "list saved devices")]
    List,
    #[command(about = "trust a device's public key")]
    Add {
        #[arg(help = "name: letters, numbers, -, _, or .")]
        name: String,
        #[arg(help = "public key file; never a private key")]
        public_key_file: PathBuf,
    },

    #[command(about = "drop a saved device")]
    Remove {
        #[arg(help = "saved peer name; restart agents to apply")]
        name: String,
    },
    #[command(about = "show a device's name, key hash, and public key")]
    Show {
        #[arg(help = "saved peer name")]
        name: String,
    },
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    #[arg(short, long)]
    pub address: String,
    #[arg(help = "name to check on the server cert")]
    #[arg(long, default_value = "localhost")]
    pub server_name: String,
    #[command(flatten)]
    pub tls: TlsFiles,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_log_commands_and_requires_app_names() {
        let cli = Cli::try_parse_from(["fleet", "logs-device", "--address", "127.0.0.1:7070", "--peer", "server", "--app", "worker", "--after", "12", "-f"]).unwrap();
        assert!(matches!(cli.command, Commands::LogsDevice { app, after: 12, follow: true, .. } if app == "worker"));
        assert!(Cli::try_parse_from(["fleet", "logs-device", "--address", "127.0.0.1:7070", "--peer", "server"]).is_err());
        assert!(Cli::try_parse_from(["fleet", "logs-device", "--address", "127.0.0.1:7070", "--peer", "server", "--app", "worker", "--after", "-1"]).is_err());
        let cli = Cli::try_parse_from(["fleet", "logs", "--address", "127.0.0.1:7070", "--app", "worker", "--ca-cert", "ca.crt", "--cert", "client.crt", "--private-key", "client.key"]).unwrap();
        assert!(matches!(cli.command, Commands::Logs { after: 0, follow: false, server_name, .. } if server_name == "localhost"));
    }
}
