use crate::workload::WorkloadSpec;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::fs::File;
use std::io;
use rustix::fs::{open, openat, Mode, OFlags};
use std::collections::HashSet;
use crate::protocol::BundleInfo;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

const MAX_FILES: usize = 256;
const MAX_FILE_BYTES: u64= 25 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 100 * 1024 * 1024;


#[derive(Debug, Deserialize, Serialize)]
pub struct DeploySpec {
    pub workload: WorkloadSpec,
    pub files: Vec<PathBuf>,
}

impl DeploySpec {
    pub fn validate(&self) -> Result<(), String> {
        self.workload.validate()?;

        if self.files.is_empty() {
            return Err(String::from("Deployment must include files"));
        }

        if self.files.len() > MAX_FILES {
            return Err(String::from("File count to big"));
        }

        let mut seen = HashSet::new();

        for file in &self.files {
            if !seen.insert(file) {
                return Err(format!("Duplicate file: {}", file.display()));
            }
        }

        Ok(())
    }

    pub fn load_manifest(path: &Path) -> Result<DeploySpec, String> {
        let file = std::fs::read_to_string(path).map_err(|e| format!("Failed to read '{}': {}", path.display(), e))?;
        let contents: DeploySpec = serde_json::from_str(&file).map_err(|e| format!("Invalid JSON in '{}': {}", path.display(), e))?;
        contents.validate()?;

        let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        contents.validate_files(parent)?;

        Ok(contents)
    }

    pub fn validate_files(&self, root: &Path) -> Result<(), String> {
        let folder_path = std::fs::canonicalize(root).map_err(|e| format!("Failed to resolve '{}': {}", root.display(), e))?;

        for file in &self.files {
            if file.as_os_str().is_empty() {
                return Err(String::from("Path cannot be empty"));
            }
            if !file.components().all(|part| matches!(part, Component::Normal(_))) {
                return Err(format!("Path must stay inside the app folder: {}", file.display()));
            }

            let mut full = folder_path.clone();

            for part in file.components() {
                full.push(part.as_os_str());
                let metadata = std::fs::symlink_metadata(&full)
                    .map_err(|e| format!("Failed to inspect '{}': {}", full.display(), e))?;
                if metadata.file_type().is_symlink() {
                    return Err(format!("Symlinks are not allowed: {}", full.display()));
                }
            }
            let metadata = std::fs::symlink_metadata(&full)
                .map_err(|e| format!("Failed to inspect '{}': {}", full.display(), e))?;

            if !metadata.is_file() {
                return Err(format!("Not a regular file: {}", full.display()));
            }
        }

        Ok(())
    }
}

pub fn bundle_info(path: &Path) -> io::Result<BundleInfo> {
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let mut file = File::from(open(path, flags, Mode::empty())?);

    if !file.metadata()?.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Bundle has to be a regular file"));
    }

    let mut sha256 = Sha256::new();
    let mut byte_counter = 0u64;
    let mut buffer = [0u8; 8192];

    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }

        byte_counter+= count as u64;

        if byte_counter > 128 * 1024 * 1024 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Bundle exceeds 128 MiB"));
        }

        sha256.update(&buffer[..count]);
    }

    Ok(BundleInfo {
        size: byte_counter,
        sha256: format!("{:x}", sha256.finalize()),
    })
}

pub fn receive_bundle(reader: &mut impl Read, info: &BundleInfo, directory: &Path) -> io::Result<PathBuf> {
    if info.size == 0 || info.size > 128 * 1024 * 1024 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid bundle size"));
    }

    let valid_hash = info.sha256.len() == 64 && info.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));

    if !valid_hash {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid bundle hash"));
    }

    let mut temp = tempfile::NamedTempFile::new_in(directory)?;
    let mut limited = reader.take(info.size);
    let mut sha256 = Sha256::new();
    let mut byte_counter = 0u64;
    let mut buffer = [0u8; 8192];

    loop {
        let count = limited.read(&mut buffer)?;
        if count == 0 {
            break;
        }

        temp.write_all(&buffer[..count])?;
        sha256.update(&buffer[..count]);
        byte_counter += count as u64;
    }

    if byte_counter != info.size {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Upload ended early"));

    }

    let hash = format!("{:x}", sha256.finalize());

    if hash != info.sha256 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Bundle hash does not match"));
    }

    let path = directory.join(format!("{}.tar", hash));
    temp.as_file().sync_all()?;
    temp.persist_noclobber(&path).map_err(|err| err.error)?;

    Ok(path)
}


pub fn package(manifest: &Path, output: &Path) -> Result<(), String> {
    let settings = DeploySpec::load_manifest(manifest)?;
    let contents: Vec<u8> = serde_json::to_vec(&settings).map_err(|e| format!("Failed to encode '{}': {}", manifest.display(), e))?;

    let parent = manifest.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let folder_path = std::fs::canonicalize(parent).map_err(|e| format!("Failed to resolve '{}': {}", parent.display(), e))?;
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;

    let root = File::from(open(&folder_path, flags, Mode::empty()).map_err(|e| format!("Failed to open '{}': {}", folder_path.display(), e))?);
    let output_parent = output.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(output_parent).map_err(|e| e.to_string())?;
    let mut archive = tar::Builder::new(temp.as_file_mut());
    let mut header = tar::Header::new_gnu();

    header.set_size(contents.len() as u64);
    header.set_mode(0o644);
    header.set_entry_type(tar::EntryType::Regular);

    archive.append_data(&mut header, "fleet.json", contents.as_slice()).map_err(|e| e.to_string())?;

    let mut total_bytes = 0u64;

    for relative in &settings.files {
        let mut file = open_source(&root, relative)
            .map_err(|e| format!("Failed to open '{}': {}", relative.display(), e))?;
        let metadata = file.metadata()
            .map_err(|e| format!("Failed to inspect '{}': {}", relative.display(), e))?;

        if metadata.len() > MAX_FILE_BYTES {
            return Err(format!("File exceeds 25 MiB: {}", relative.display()));
        }
        total_bytes += metadata.len();

        if total_bytes > MAX_TOTAL_BYTES {
            return Err(format!("Combined files exceed 100 MiB: {}", relative.display()));
        }
        archive.append_file(Path::new("app").join(relative), &mut file)
            .map_err(|e| format!("Failed to add '{}': {}", relative.display(), e))?;
    }

    archive.finish().map_err(|e| e.to_string())?;
    drop(archive);
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist_noclobber(output).map_err(|e| format!("Failed to save '{}': {}", output.display(), e))?;
    Ok(())
}

fn open_source(root: &File, relative: &Path) -> io::Result<File> {
    let mut directory = root.try_clone()?;
    let mut parts = relative.components().peekable();

    while let Some(part) = parts.next() {
        let name = match part {
            Component::Normal(name) => name,
            _ => return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid file path")),
        };
        let last = parts.peek().is_none();
        let mut flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        flags |= if last { OFlags::NONBLOCK } else { OFlags::DIRECTORY };

        let file = File::from(openat(&directory, name, flags, Mode::empty())?);
        if last {
            if !file.metadata()?.is_file() {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "Not a regular file"));
            }
            return Ok(file);
        }
        directory = file;
    }
    Err(io::Error::new(io::ErrorKind::InvalidInput, "Path cannot be empty"))
}

#[cfg(test)]
mod tests;
