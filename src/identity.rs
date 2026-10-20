use p256::pkcs8::{DecodePublicKey, EncodePublicKey};
use rcgen::KeyPair;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::sign::SigningKey;
use sha2::{Digest, Sha256};
use std::io;
use std::sync::Arc;
#[cfg(unix)]
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};

#[cfg(unix)]
const MAX_KEY_BYTES: u64 = 16 * 1024;

pub struct DeviceIdentity {
    key_pair: KeyPair,
}

impl DeviceIdentity {
    pub fn generate() -> io::Result<Self> {
        let key_pair = KeyPair::generate().map_err(io::Error::other)?;

        Ok(Self { key_pair })
    }

    pub fn public_key_pem(&self) -> String {
        self.key_pair.public_key_pem()
    }

    #[cfg(unix)]
    pub fn load(path: &Path) -> io::Result<Self> {
        let pem = read_private(path, MAX_KEY_BYTES)?;
        let key_pair = KeyPair::from_pem(&pem).map_err(io::Error::other)?;

        Ok(Self { key_pair })
    }

    #[cfg(unix)]
    pub fn save_new(&self, path: &Path) -> io::Result<()> {
        let pem = self.key_pair.serialize_pem();
        write_private(path, pem.as_bytes(), false)
    }

    #[cfg(unix)]
    pub fn load_or_create(path: &Path) -> io::Result<Self> {
        match Self::load(path) {
            Ok(identity) => Ok(identity),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let identity = Self::generate()?;
                match identity.save_new(path) {
                    Ok(()) => Ok(identity),
                    // reuse the key if another start wrote it
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Self::load(path),
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    }

    pub fn load_default() -> io::Result<Self> {
        #[cfg(unix)]
        {
            let home = std::env::var_os("HOME")
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
            Self::load_for_home(Path::new(&home))
        }
        #[cfg(not(unix))]
        {
            //todo: add windows key files
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Automatic identity storage currently supports macOS and Linux only",
            ))
        }
    }

    #[cfg(unix)]
    fn load_for_home(home: &Path) -> io::Result<Self> {
        let directory = prepare_directory(home)?;
        Self::load_or_create(&directory.join("device-key.pem"))
    }

    pub fn signing_key(&self) -> io::Result<Arc<dyn SigningKey>> {
        let der = self.key_pair.serialize_der();
        let key: PrivateKeyDer<'static> = PrivatePkcs8KeyDer::from(der).into();
        let provider = rustls::crypto::aws_lc_rs::default_provider();

        provider
            .key_provider
            .load_private_key(key)
            .map_err(io::Error::other)
    }
}
pub(crate) fn fingerprint(public_key: &str) -> io::Result<String> {
    let key = p256::PublicKey::from_public_key_pem(public_key.trim()).map_err(io::Error::other)?;
    let der = key.to_public_key_der().map_err(io::Error::other)?;

    let hash = Sha256::digest(der.as_bytes());

    Ok(format!("SHA256:{:x}", hash))
}

pub(crate) fn default_directory() -> io::Result<std::path::PathBuf> {
    #[cfg(unix)]
    {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
        prepare_directory(Path::new(&home))
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
fn prepare_directory(home: &Path) -> io::Result<std::path::PathBuf> {
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
    Ok(directory)
}

#[cfg(unix)]
fn parent_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

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

#[cfg(unix)]
pub(crate) fn write_private(path: &Path, contents: &[u8], replace: bool) -> io::Result<()> {
    let parent = parent_directory(path);
    check_directory(parent, true)?;
    if replace {
        match read_private(path, 1024 * 1024) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(contents)?;
    file.as_file().sync_all()?;
    let saved = if replace {
        file.persist(path)
    } else {
        file.persist_noclobber(path)
    };
    saved.map_err(|error| error.error)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(unix)]
pub(crate) fn read_private(path: &Path, max_bytes: u64) -> io::Result<String> {
    check_directory(parent_directory(path), true)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Private storage must be a regular file",
        ));
    }
    if metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Private file must belong to you and have owner-only permissions",
        ));
    }
    let mut pem = String::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_string(&mut pem)?;
    if pem.len() as u64 > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Private file exceeds its size limit",
        ));
    }
    Ok(pem)
}

#[cfg(unix)]
pub(crate) fn lock_private(path: &Path) -> io::Result<std::fs::File> {
    check_directory(parent_directory(path), true)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Lock file must be a regular, owner-only file belonging to you",
        ));
    }
    file.lock()?;
    Ok(file)
}

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
