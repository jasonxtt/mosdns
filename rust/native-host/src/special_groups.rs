//! Small helpers shared by the configuration compiler and the apply path: the set of files a
//! compile read (checked again at the commit point), a lexical path normaliser, and SHA-256.
//! (The experimental `native_management` profile that used to live here was removed.)
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Every canonical input read while compiling one managed candidate. Paths
/// remain lexical so a symlink retarget or an added formerly-missing optional
/// file is checked again before the transaction replaces canonical state.
#[derive(Clone, Debug, Default)]
pub(crate) struct CandidateInputSet {
    pub files: BTreeMap<PathBuf, Option<String>>,
}

impl CandidateInputSet {
    pub(crate) fn record_bytes(&mut self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        self.record(path, Some(sha256(bytes)))
    }

    pub(crate) fn record_missing(&mut self, path: &Path) -> Result<(), String> {
        self.record(path, None)
    }

    fn record(&mut self, path: &Path, digest: Option<String>) -> Result<(), String> {
        let path = lexical_path(path);
        if let Some(previous) = self.files.get(&path) {
            if previous != &digest {
                return Err(format!(
                    "input changed while compiling candidate: {}",
                    path.display()
                ));
            }
        } else {
            self.files.insert(path, digest);
        }
        Ok(())
    }
}

pub(crate) fn lexical_path(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                output.pop();
            }
            component => output.push(component.as_os_str()),
        }
    }
    output
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
