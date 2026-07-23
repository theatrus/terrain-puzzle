use std::{
    env,
    ffi::OsStr,
    fs,
    fs::OpenOptions,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use uuid::Uuid;

pub fn root() -> Result<PathBuf> {
    if let Some(value) = env::var_os("TERRAIN_CACHE_DIR").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(value));
    }
    ProjectDirs::from("com", "theatrus", "terrain-puzzle")
        .map(|directories| directories.cache_dir().to_path_buf())
        .context("find the OS cache directory; set TERRAIN_CACHE_DIR to choose one")
}

pub fn store(path: &Path, bytes: &[u8]) -> Result<()> {
    store_reader(path, bytes)
}

pub fn store_reader(path: &Path, mut reader: impl Read) -> Result<()> {
    if path.is_file() {
        return Ok(());
    }
    let parent = path
        .parent()
        .context("cached input path has no parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create input cache directory {}", parent.display()))?;
    let file_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .context("cached input path has no file name")?;
    let temporary = parent.join(format!(".{file_name}.{}.part", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .with_context(|| format!("create temporary cache file {}", temporary.display()))?;
        std::io::copy(&mut reader, &mut file)
            .with_context(|| format!("write temporary cache file {}", temporary.display()))?;
        file.flush()?;
        file.sync_all()?;
        if path.is_file() {
            return Ok(());
        }
        match fs::rename(&temporary, path) {
            Ok(()) => Ok(()),
            Err(_) if path.is_file() => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("publish cached input {}", path.display()))
            }
        }
    })();
    if temporary.exists() {
        fs::remove_file(&temporary)
            .with_context(|| format!("remove temporary cache file {}", temporary.display()))?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomically_stores_cached_input() {
        let directory =
            std::env::temp_dir().join(format!("terrain-puzzle-cache-test-{}", Uuid::new_v4()));
        let path = directory.join("tiles").join("sample.bin");
        store(&path, b"first").unwrap();
        store(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first");
        fs::remove_dir_all(directory).unwrap();
    }
}
