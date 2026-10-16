use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

const SIGNATURE: &[u8] = b"# GRUB Environment Block\n";
const DEFAULT_SIZE: usize = 1024;

type Entries = Vec<(String, String)>;

#[derive(Debug, Clone)]
pub struct GrubEnv {
    path: PathBuf,
    entries: Entries,
    size: usize,
}

impl GrubEnv {
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Self> {
        let env = Self {
            path: path.as_ref().to_path_buf(),
            entries: Vec::new(),
            size: DEFAULT_SIZE,
        };
        write_atomic(&env.path, &encode(&env.entries, env.size)?)?;
        Ok(env)
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let data = fs::read(path.as_ref())?;
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            entries: decode(&data)?,
            size: data.len(),
        })
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn set(&mut self, name: &str, value: &str) -> Result<()> {
        validate_name(name)?;
        validate_value(name, value)?;

        let mut entries = self.entries.clone();
        upsert(&mut entries, name.to_owned(), value.to_owned());
        self.commit(entries)
    }

    pub fn unset(&mut self, name: &str) -> Result<()> {
        if self.get(name).is_none() {
            return Ok(());
        }
        let entries = self
            .entries
            .iter()
            .filter(|(n, _)| n != name)
            .cloned()
            .collect();
        self.commit(entries)
    }

    pub fn list(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(n, v)| (n.as_str(), v.as_str()))
    }

    fn commit(&mut self, entries: Entries) -> Result<()> {
        write_atomic(&self.path, &encode(&entries, self.size)?)?;
        self.entries = entries;
        Ok(())
    }
}

fn upsert(entries: &mut Entries, name: String, value: String) {
    match entries.iter_mut().find(|(n, _)| *n == name) {
        Some(entry) => entry.1 = value,
        None => entries.push((name, value)),
    }
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.starts_with('#')
        || name.bytes().any(|b| b == b'=' || b == b'\n' || b == 0)
    {
        return Err(Error::InvalidName(name.to_owned()));
    }
    Ok(())
}

fn validate_value(name: &str, value: &str) -> Result<()> {
    if value.bytes().any(|b| b == 0) {
        return Err(Error::InvalidValue(name.to_owned()));
    }
    Ok(())
}

fn encode(entries: &Entries, size: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(SIGNATURE);
    for (name, value) in entries {
        out.extend_from_slice(name.as_bytes());
        out.push(b'=');
        for &b in value.as_bytes() {
            if b == b'\\' || b == b'\n' {
                out.push(b'\\');
            }
            out.push(b);
        }
        out.push(b'\n');
    }
    if out.len() > size {
        return Err(Error::NoSpace {
            required: out.len(),
            capacity: size,
        });
    }
    out.resize(size, b'#');
    Ok(out)
}

fn decode(data: &[u8]) -> Result<Entries> {
    if !data.starts_with(SIGNATURE) {
        return Err(Error::InvalidSignature);
    }

    let mut entries = Vec::new();
    let mut pos = SIGNATURE.len();

    while pos < data.len() {
        match data[pos] {
            0 => break,
            b'\n' => pos += 1,
            b'#' => {
                pos = data[pos..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map_or(data.len(), |i| pos + i + 1);
            }
            _ => {
                let start = pos;
                let eq = data[pos..]
                    .iter()
                    .position(|&b| b == b'=' || b == b'\n')
                    .map(|i| pos + i)
                    .filter(|&i| data[i] == b'=')
                    .ok_or(Error::Malformed { offset: start })?;
                let name = std::str::from_utf8(&data[start..eq])
                    .map_err(|_| Error::InvalidUtf8)?
                    .to_owned();
                pos = eq + 1;

                let mut raw = Vec::new();
                while pos < data.len() {
                    match data[pos] {
                        b'\\' if pos + 1 < data.len() => {
                            raw.push(data[pos + 1]);
                            pos += 2;
                        }
                        b'\\' => pos += 1,
                        b'\n' => {
                            pos += 1;
                            break;
                        }
                        b => {
                            raw.push(b);
                            pos += 1;
                        }
                    }
                }
                let value = String::from_utf8(raw).map_err(|_| Error::InvalidUtf8)?;

                if validate_name(&name).is_err() || validate_value(&name, &value).is_err() {
                    return Err(Error::Malformed { offset: start });
                }
                upsert(&mut entries, name, value);
            }
        }
    }

    Ok(entries)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "grubenv".to_owned());
    let tmp_name = format!(".{}.{}.tmp", name, std::process::id());
    let tmp = match target.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(tmp_name),
        _ => PathBuf::from(tmp_name),
    };

    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if let Ok(meta) = fs::metadata(&target) {
            fs::set_permissions(&tmp, meta.permissions())?;
        }
        fs::rename(&tmp, &target)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
