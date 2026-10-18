//! Stores device keys approved for the device-key agent and client commands.
use crate::cli::PeerCommands;
use p256::pkcs8::{DecodePublicKey, EncodePublicKey, LineEnding};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

const MAX_KEY_BYTES: usize = 16 * 1024;
const MAX_STORE_BYTES: usize = 1024 * 1024;

/// A device label and its public key, never its private key.
#[derive(Serialize, Deserialize)]
pub struct TrustedPeer {
    /// A unique, printable device label.
    pub name: String,
    /// A validated P-256 public key in a consistent PEM format.
    pub public_key: String,
}

/// Stores peers without duplicate names or public keys.
pub struct TrustStore {
    peers: Vec<TrustedPeer>,
}

impl TrustStore {
    /// Creates an empty store.
    pub fn new() -> Self {
        Self { peers: Vec::new() }
    }

    /// Adds a valid P-256 public key, rejecting unsafe names and duplicates.
    pub fn add(&mut self, mut peer: TrustedPeer) -> Result<(), String> {
        if peer.name.is_empty()
            || peer.name.len() > 64
            || !peer
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        {
            return Err(String::from(
                "Peer name must be 1-64 letters, digits, hyphens, underscores, or dots",
            ));
        }
        peer.public_key =
            normalize_public_key(&peer.public_key).map_err(|error| error.to_string())?;
        if self
            .peers
            .iter()
            .any(|existing| existing.public_key == peer.public_key)
        {
            return Err(String::from("Public key is already trusted"));
        }
        if self.peers.iter().any(|existing| existing.name == peer.name) {
            return Err(String::from("Peer name is already used"));
        }
        self.peers.push(peer);
        Ok(())
    }

    /// Checks a public key, ignoring PEM formatting differences.
    pub fn contains(&self, public_key: &str) -> bool {
        match normalize_public_key(public_key) {
            Ok(key) => self.peers.iter().any(|peer| peer.public_key == key),
            Err(_) => false,
        }
    }

    /// Encodes the peer list as JSON without private keys.
    pub fn to_json(&self) -> io::Result<String> {
        serde_json::to_string_pretty(&self.peers).map_err(io::Error::other)
    }

    /// Decodes JSON and checks every name and public key.
    pub fn from_json(contents: &str) -> io::Result<Self> {
        if contents.len() > MAX_STORE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Peer list exceeds 1 MiB",
            ));
        }
        let parsed: Vec<TrustedPeer> = serde_json::from_str(contents).map_err(io::Error::other)?;
        let mut store = Self::new();

        for peer in parsed {
            store.add(peer).map_err(io::Error::other)?;
        }

        Ok(store)
    }

    /// Loads a private file, treating a missing file as an empty list.
    #[cfg(unix)]
    pub fn load(path: &Path) -> io::Result<Self> {
        match crate::identity::read_private(path, MAX_STORE_BYTES as u64) {
            Ok(content) => Self::from_json(&content),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::new()),
            Err(error) => Err(error),
        }
    }

    /// Saves a complete private file. Callers must lock around load, change, and save.
    #[cfg(unix)]
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let json = self.to_json()?;
        if json.len() > MAX_STORE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "JSON payload exceeds the 1 MiB limit",
            ));
        }
        crate::identity::write_private(path, json.as_bytes(), true)
    }

    /// Borrows the approved peers without allowing changes.
    pub fn peers(&self) -> &[TrustedPeer] {
        &self.peers
    }

    /// Removes a named peer, returning whether one was found.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.peers.len();
        self.peers.retain(|peer| peer.name != name);

        self.peers.len() != before
    }
}

/// Lists, shows, or changes saved peers while holding a separate file lock.
/// Does not need certificates or a token and does not change network access yet.
///
/// # Errors
/// Returns file, validation, or locking errors. Unsupported platforms return an error.
pub fn run(command: PeerCommands) -> io::Result<()> {
    #[cfg(unix)]
    {
        let directory = crate::identity::default_directory()?;
        let _lock = crate::identity::lock_private(&directory.join("trusted-peers.lock"))?;
        let path = directory.join("trusted-peers.json");
        let mut store = TrustStore::load(&path)?;
        match command {
            PeerCommands::List => {
                if store.peers().is_empty() {
                    println!("No approved peers.");
                }
                for peer in store.peers() {
                    println!("{}", peer.name);
                }
            }
            PeerCommands::Add {
                name,
                public_key_file,
            } => {
                let public_key = read_public_key(&public_key_file)?;
                store
                    .add(TrustedPeer {
                        name: name.clone(),
                        public_key,
                    })
                    .map_err(io::Error::other)?;
                store.save(&path)?;
                println!("Added peer: {}", name);
            }
            PeerCommands::Remove { name } => {
                if !store.remove(&name) {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        "Peer name was not found",
                    ));
                }
                store.save(&path)?;
                println!("Removed peer: {}", name);
            }
            PeerCommands::Show { name } => {
                match store.peers().iter().find(|peer| peer.name == name) {
                    Some(peer) => {
                        let fingerprint = crate::identity::fingerprint(&peer.public_key)?;
                        println!("Device name: {}", peer.name);
                        println!("Fingerprint: {}", fingerprint);
                        print!("Public key:\n{}", peer.public_key);
                    }
                    None => {
                        return Err(io::Error::new(
                            io::ErrorKind::NotFound,
                            "Device was not found",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = command;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Peer storage currently supports macOS and Linux only",
        ))
    }
}

/// Validates Fleet's P-256 public key and stores it in one consistent PEM format.
fn normalize_public_key(contents: &str) -> io::Result<String> {
    if contents.len() > MAX_KEY_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Public key exceeds 16 KiB",
        ));
    }
    let key = p256::PublicKey::from_public_key_pem(contents.trim()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Expected one valid P-256 PUBLIC KEY PEM, not a private key",
        )
    })?;
    key.to_public_key_pem(LineEnding::LF)
        .map_err(io::Error::other)
}

/// Reads a bounded public-key file without following symlinks or opening devices.
#[cfg(unix)]
fn read_public_key(path: &Path) -> io::Result<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Public key must be a regular file",
        ));
    }
    let mut contents = String::new();
    file.take(MAX_KEY_BYTES as u64 + 1)
        .read_to_string(&mut contents)?;
    normalize_public_key(&contents)
}

#[cfg(test)]
mod tests;
