use crate::client;
use crate::tls::TlsFiles;
use serde::Deserialize;
use std::io;

#[derive(Debug, Deserialize)]
pub struct NodeSpec {
    pub name: String,
    pub address: String,
    pub server_name: String,

    #[serde(default)]
    pub peer: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClusterSpec {
    pub nodes: Vec<NodeSpec>,
}

pub fn watch(cluster: &ClusterSpec, files: &TlsFiles) -> io::Result<()> {
    let client = client::FleetClient::new(files)?;
    let clients = vec![client; cluster.nodes.len()];
    watch_clients(cluster, &clients)
}

pub fn watch_device(cluster: &ClusterSpec) -> io::Result<()> {
    let clients = cluster.device_clients()?;
    watch_clients(cluster, &clients)
}

fn watch_clients(cluster: &ClusterSpec, clients: &[client::FleetClient]) -> io::Result<()> {
    cluster.validate().map_err(io::Error::other)?;
    if cluster.nodes.len() != clients.len() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Each node needs one client"));
    }

    loop {
        // check up to four nodes at once
        for (nodes, clients) in cluster.nodes.chunks(4).zip(clients.chunks(4)) {
            std::thread::scope(|scope| {
                for (node, client) in nodes.iter().zip(clients.iter()) {
                    scope.spawn(move || check_node(node, client));
                }
            });
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

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

    pub fn device_clients(&self) -> io::Result<Vec<client::FleetClient>> {
        self.validate().map_err(io::Error::other)?;
        let mut clients = Vec::new();

        for node in &self.nodes {
            let peer = match &node.peer {
                Some(peer) if !peer.trim().is_empty() => peer,
                _ => {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("Node '{}' needs a nonblank peer name", node.name)));
                }

            };
            clients.push(client::FleetClient::for_peer(peer)?);
        }

        Ok(clients)
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
            peer: None,
        }
    }

    #[test]
    fn watch_rejects_missing_clients_before_polling() {
        let cluster = ClusterSpec { nodes: vec![node("127.0.0.1:7070")] };
        let error = watch_clients(&cluster, &[]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(error.to_string(), "Each node needs one client");
    }

    #[test]
    fn watch_rejects_empty_clusters_before_polling() {
        let cluster = ClusterSpec { nodes: Vec::new() };
        assert!(watch_clients(&cluster, &[]).is_err());
    }

    #[test]
    fn device_clients_reject_missing_and_blank_peer_names() {
        for peer in [None, Some(String::new()), Some(String::from(" \t"))] {
            let mut node = node("127.0.0.1:7070");
            node.peer = peer;
            let cluster = ClusterSpec { nodes: vec![node] };
            let error = match cluster.device_clients() {
                Ok(_) => panic!("accepted missing or blank peer"),
                Err(error) => error,
            };
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert!(error.to_string().contains("test-agent"));
        }
    }

    #[test]
    fn certificate_manifests_can_omit_peer_names() {
        let cluster: ClusterSpec = serde_json::from_str(include_str!("../examples/cluster.json")).unwrap();
        assert!(cluster.nodes.iter().all(|node| node.peer.is_none()));
        cluster.validate().unwrap();
    }

    #[test]
    fn device_example_has_separate_peer_names() {
        let cluster: ClusterSpec = serde_json::from_str(include_str!("../examples/cluster-device.json")).unwrap();
        cluster.validate().unwrap();
        assert_eq!(cluster.nodes.len(), 2);
        assert_eq!(cluster.nodes[0].peer.as_deref(), Some("local-one"));
        assert_eq!(cluster.nodes[1].peer.as_deref(), Some("local-two"));
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
