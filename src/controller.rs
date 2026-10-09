//! Checks the status of agents listed in a cluster file.

use crate::client;
use crate::tls::TlsFiles;
use serde::Deserialize;
use std::io;

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

/// Checks agents in batches of up to four and pauses two seconds after each round.
///
/// Failed checks are printed without ending the loop. Each batch runs its checks
/// together and finishes before the next batch starts. Ctrl+C ends this watcher,
/// not the agents or their workloads.
/// One client shares the loaded token and TLS config across all checks.
/// Restart the watcher to load changed credentials. Each check opens a new connection.
/// This function does not start, stop, or move workloads.
///
/// # Errors
/// Returns an error before polling if the token is missing or blank, or the local
/// TLS files cannot be loaded. Agents check the token and certificates on connection.
pub fn watch(cluster: &ClusterSpec, files: &TlsFiles) -> io::Result<()> {
    let client = client::FleetClient::new(files)?;

    loop {
        for batch in cluster.nodes.chunks(4) {
            std::thread::scope(|scope| {
                let client = &client;
                for node in batch {
                    scope.spawn(move || check_node(node, client));
                }
            });
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

/// Checks one agent and prints its counts or an error.
pub fn check_node(node: &NodeSpec, client: &client::FleetClient) {
    match client.fetch_status(&node.address, &node.server_name) {
        Ok(status) => println!(
            "{}: running {}, desired {}",
            node.name, status.running, status.desired
        ),
        Err(error) => eprintln!("{}: check failed: {}", node.name, error),
    }
}

impl ClusterSpec {
    /// Checks that the cluster has nodes with names and usable IP addresses and ports.
    ///
    /// This does not contact agents or check their certificates.
    ///
    /// # Errors
    /// Returns a message if the cluster is empty, a name is blank, or an address
    /// is invalid, has port zero, or uses a wildcard IP.
    pub fn validate(&self) -> Result<(), String> {
        if self.nodes.is_empty() {
            return Err("Cluster has to have at least one node.".to_string());
        }

        for node in &self.nodes {
            if node.name.trim().is_empty() {
                return Err("Node name cannot be blank.".to_string());
            }
            if node.server_name.trim().is_empty() {
                return Err("Cluster cannot have empty server name.".to_string());
            }
            let address: std::net::SocketAddr = node
                .address
                .parse()
                .map_err(|error| format!("Invalid address for {}: {}", node.name, error))?;

            if (address.port() == 0) || (address.ip().is_unspecified()) {
                return Err("Address port or IP is unspecified".to_string());
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(address: &str) -> NodeSpec {
        NodeSpec {
            name: String::from("test-agent"),
            address: String::from(address),
            server_name: String::from("localhost"),
        }
    }

    #[test]
    fn accepts_ipv4_and_ipv6_nodes() {
        let cluster = ClusterSpec {
            nodes: vec![node("127.0.0.1:7070"), node("[::1]:7071")],
        };
        assert!(cluster.validate().is_ok());
    }

    #[test]
    fn accepts_remote_addresses() {
        let cluster = ClusterSpec {
            nodes: vec![node("100.64.0.1:7070"), node("[2001:db8::1]:65535")],
        };
        assert!(cluster.validate().is_ok());
    }

    #[test]
    fn rejects_empty_clusters() {
        let cluster = ClusterSpec { nodes: Vec::new() };
        assert!(cluster.validate().is_err());
    }

    #[test]
    fn rejects_blank_node_names() {
        for name in ["", " \t\n"] {
            let mut agent = node("127.0.0.1:7070");
            agent.name = String::from(name);
            let cluster = ClusterSpec { nodes: vec![agent] };
            assert!(cluster.validate().is_err(), "accepted name: {name:?}");
        }
    }

    #[test]
    fn rejects_blank_server_names() {
        for server_name in ["", " \t\n"] {
            let mut agent = node("127.0.0.1:7070");
            agent.server_name = String::from(server_name);
            let cluster = ClusterSpec { nodes: vec![agent] };
            assert!(
                cluster.validate().is_err(),
                "accepted server name: {server_name:?}"
            );
        }
    }

    #[test]
    fn rejects_invalid_addresses() {
        for address in [
            "",
            "not-an-address",
            "localhost:7070",
            "127.0.0.1",
            "127.0.0.1:70000",
            "::1:7070",
        ] {
            let cluster = ClusterSpec {
                nodes: vec![node(address)],
            };
            assert!(cluster.validate().is_err(), "accepted address: {address}");
        }
    }

    #[test]
    fn rejects_port_zero_and_wildcard_ips() {
        for address in ["127.0.0.1:0", "[::1]:0", "0.0.0.0:7070", "[::]:7070"] {
            let cluster = ClusterSpec {
                nodes: vec![node(address)],
            };
            assert!(cluster.validate().is_err(), "accepted address: {address}");
        }
    }

    #[test]
    fn checks_nodes_after_the_first() {
        let cluster = ClusterSpec {
            nodes: vec![node("127.0.0.1:7070"), node("[::]:7071")],
        };
        assert!(cluster.validate().is_err());
    }
}
