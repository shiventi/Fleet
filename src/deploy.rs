use crate::workload::WorkloadSpec;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::fs::File;
use std::io;
use rustix::fs::{open, openat, Mode, OFlags};

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

    for relative in &settings.files {
        let mut file = open_source(&root, relative)
            .map_err(|e| format!("Failed to open '{}': {}", relative.display(), e))?;
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
