use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use aelys_syntax::Source;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const VERSION: u32 = 2;

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

struct Stat {
    size: u64,
    mtime: i64,
}

fn stat(path: &Path) -> io::Result<Stat> {
    let metadata = std::fs::metadata(path)?;
    let mtime = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos() as i64);
    Ok(Stat {
        size: metadata.len(),
        mtime,
    })
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> io::Result<String> {
    Ok(sha256(&std::fs::read(path)?))
}

pub fn is_fresh(record: &Path, key: &Key) -> bool {
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
    old.input.iter().all(|input| {
        let path = Path::new(&input.path);
        stat(path).is_ok_and(|now| now.size == input.size && now.mtime == input.mtime)
            || sha256_file(path).is_ok_and(|hash| hash == input.sha256)
    })
}

pub fn record(record: &Path, key: Key, files: &[(PathBuf, Arc<Source>)]) -> io::Result<()> {
    let mut input = Vec::with_capacity(files.len());
    for (path, source) in files {
        let now = stat(path)?;
        let sha256 = sha256_file(path)?;
        // the file changed while it was being compiled, so no record may vouch for the artifact
        if sha256 != self::sha256(source.content.as_bytes()) {
            return match std::fs::remove_file(record) {
                Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
                _ => Ok(()),
            };
        }
        input.push(Input {
            path: path.to_string_lossy().into_owned(),
            size: now.size,
            mtime: now.mtime,
            sha256,
        });
    }
    let text = toml::to_string(&Record {
        version: VERSION,
        key,
        input,
    })
    .map_err(io::Error::other)?;
    if let Some(dir) = record.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temporary = record.with_extension("toml.tmp");
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, record)
}
