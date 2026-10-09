//! Checks the status of agents listed in a cluster file.

use crate::client;
use crate::tls::TlsFiles;
use serde::Deserialize;

/// One agent to check.
#[derive(Debug, Deserialize)]
pub struct NodeSpec {
    /// A label shown in the watcher output.
    pub name: String,
    /// The agent's numeric IP address and port.
    pub address: String,
    /// The name expected in the agent's certificate.
    pub server_name: String,
}

/// The agents listed in a cluster file.
#[derive(Debug, Deserialize)]
pub struct ClusterSpec {
    /// Agents checked during each round.
    pub nodes: Vec<NodeSpec>,
}

/// Checks agents one at a time and pauses two seconds after each round.
///
/// Failed checks are printed without ending the loop. A slow check delays the
/// next node. Ctrl+C ends this watcher, not the agents or their workloads.
/// This function does not start, stop, or move workloads.
pub fn watch(cluster: &ClusterSpec, files: &TlsFiles) {
    loop {
        for node in &cluster.nodes {
            match client::fetch_status(&node.address, &node.server_name, files) {
                Ok(status) => println!(
                    "{}: running {}, desired {}",
                    node.name, status.running, status.desired
                ),
                Err(error) => eprintln!("{}: check failed: {}", node.name, error),
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}
