//! Creates and reuses this device's identity. Not connected to authentication yet.

use rcgen::KeyPair;
use std::io;
#[cfg(unix)]
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};

#[cfg(unix)]
const MAX_KEY_BYTES: u64 = 16 * 1024;

/// Holds a device's signing keys. Never print or share the private key.
pub struct DeviceIdentity {
    key_pair: KeyPair,
}

impl DeviceIdentity {
    /// Generates an unsaved identity, returning any key-generation error.
    pub fn generate() -> io::Result<Self> {
        let key_pair = KeyPair::generate().map_err(io::Error::other)?;

        Ok(Self { key_pair })
    }

    /// Returns only the public key as readable PEM text.
    pub fn public_key_pem(&self) -> String {
        self.key_pair.public_key_pem()
    }

    /// Loads a private key, rejecting unsafe files, invalid keys, or read errors.
    #[cfg(unix)]
    pub fn load(path: &Path) -> io::Result<Self> {
        check_directory(parent_directory(path), true)?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(
                (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
            )
            .open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Identity must be a regular file",
            ));
        }
        if metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Identity file must belong to you and have owner-only permissions",
            ));
        }
        let mut pem = String::new();
        file.take(MAX_KEY_BYTES + 1).read_to_string(&mut pem)?;
        if pem.len() as u64 > MAX_KEY_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Identity file exceeds 16 KiB",
            ));
        }
        let key_pair = KeyPair::from_pem(&pem).map_err(io::Error::other)?;

        Ok(Self { key_pair })
    }

    /// Saves a complete key without overwriting. Requires a private parent directory.
    /// Returns file, permission, or sync errors.
    #[cfg(unix)]
    pub fn save_new(&self, path: &Path) -> io::Result<()> {
        let parent = parent_directory(path);
        check_directory(parent, true)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(self.key_pair.serialize_pem().as_bytes())?;
        file.as_file().sync_all()?;
        file.persist_noclobber(path).map_err(|error| error.error)?;
        std::fs::File::open(parent)?.sync_all()?;

        Ok(())
    }

    /// Reuses a saved key or creates a missing one. Other errors are returned unchanged.
    #[cfg(unix)]
    pub fn load_or_create(path: &Path) -> io::Result<Self> {
        match Self::load(path) {
            Ok(identity) => Ok(identity),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let identity = Self::generate()?;
                match identity.save_new(path) {
                    Ok(()) => Ok(identity),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Self::load(path),
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    }

    /// Creates private storage in ~/.config/fleet/identity and reuses its key.
    /// Stores device-key.pem and never prints the private key.
    /// Returns storage errors; non-Unix platforms are not supported yet.
    pub fn load_default() -> io::Result<Self> {
        #[cfg(unix)]
        {
            let home = std::env::var_os("HOME")
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
            Self::load_for_home(Path::new(&home))
        }
        #[cfg(not(unix))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Automatic identity storage currently supports macOS and Linux only",
            ))
        }
    }

    #[cfg(unix)]
    fn load_for_home(home: &Path) -> io::Result<Self> {
        if !home.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "HOME must be an absolute directory path",
            ));
        }
        check_directory(home, false)?;
        let mut directory = home.to_path_buf();
        for name in [".config", "fleet", "identity"] {
            directory.push(name);
            ensure_directory(&directory, name == "identity")?;
        }
        Self::load_or_create(&directory.join("device-key.pem"))
    }
}

#[cfg(unix)]
fn parent_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// Rejects symlinks and directories that another user could change.
#[cfg(unix)]
fn check_directory(path: &Path, private: bool) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "Identity storage must use a real directory: {}",
                path.display()
            ),
        ));
    }
    let forbidden = if private { 0o077 } else { 0o022 };
    if metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.permissions().mode() & forbidden != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "Identity directory must belong to you and have safe permissions: {}",
                path.display()
            ),
        ));
    }
    Ok(())
}

/// New directories are private; existing unsafe directories are rejected, not modified.
#[cfg(unix)]
fn ensure_directory(path: &Path, private: bool) -> io::Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => std::fs::File::open(parent_directory(path))?.sync_all()?,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    check_directory(path, private)
}

#[cfg(all(test, unix))]
mod tests;
