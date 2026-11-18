use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct Key {
    pub compiler: String,
    pub std: String,
    pub core_lib: String,
    pub options: String,
    pub artifact: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Record {
    version: u32,
    key: Key,
    input: Vec<Input>,
}

#[derive(Serialize, Deserialize)]
struct Input {
    path: String,
    size: u64,
    mtime: i64,
    sha256: String,
}

pub struct Snapshot {
    path: PathBuf,
    size: u64,
    mtime: i64,
}

pub fn scan(dir: &Path, files: &[&Path]) -> io::Result<Vec<Snapshot>> {
    let mut paths: Vec<PathBuf> = files.iter().map(|file| file.to_path_buf()).collect();
    collect_sources(dir, &mut paths)?;
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .map(|path| {
            let metadata = std::fs::metadata(&path)?;
            let mtime = metadata
                .modified()?
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_nanos() as i64);
            Ok(Snapshot {
                path,
                size: metadata.len(),
                mtime,
            })
        })
        .collect()
}

fn collect_sources(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_sources(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "aelys") {
            out.push(path);
        }
    }
    Ok(())
}

fn sha256(path: &Path) -> io::Result<String> {
    Ok(format!("{:x}", Sha256::digest(std::fs::read(path)?)))
}

fn display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub fn is_fresh(record: &Path, key: &Key, inputs: &[Snapshot]) -> bool {
    let Some(old) = std::fs::read_to_string(record)
        .ok()
        .and_then(|text| toml::from_str::<Record>(&text).ok())
    else {
        return false;
    };
    if old.version != VERSION || old.key != *key {
        return false;
    }
    if key
        .artifact
        .as_ref()
        .is_some_and(|artifact| !Path::new(artifact).is_file())
    {
        return false;
    }
    old.input.len() == inputs.len()
        && old.input.iter().zip(inputs).all(|(old, now)| {
            old.path == display(&now.path)
                && ((old.size == now.size && old.mtime == now.mtime)
                    || sha256(&now.path).is_ok_and(|hash| hash == old.sha256))
        })
}

pub struct Pending(Record);

pub fn snapshot(key: Key, inputs: &[Snapshot]) -> io::Result<Pending> {
    let input = inputs
        .iter()
        .map(|now| {
            Ok(Input {
                path: display(&now.path),
                size: now.size,
                mtime: now.mtime,
                sha256: sha256(&now.path)?,
            })
        })
        .collect::<io::Result<_>>()?;
    Ok(Pending(Record {
        version: VERSION,
        key,
        input,
    }))
}

impl Pending {
    pub fn commit(&self, record: &Path) -> io::Result<()> {
        let text = toml::to_string(&self.0).map_err(io::Error::other)?;
        if let Some(dir) = record.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temporary = record.with_extension("toml.tmp");
        std::fs::write(&temporary, text)?;
        std::fs::rename(&temporary, record)
    }
}
