use crate::project::config::ProjectConfig;
use crate::project::source::{
    collect_sources_with_config, read_project_bytes, safe_project_input_path,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::time::SystemTime;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Stamp {
    modified: SystemTime,
    size: u64,
    digest: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub files: BTreeMap<String, Option<Stamp>>,
    source_paths: std::collections::BTreeSet<String>,
    root_created: Option<SystemTime>,
    #[cfg(unix)]
    root_identity: (u64, u64),
    pub graph: Option<Vec<u8>>,
}

pub(super) fn exclusions() -> Vec<String> {
    ["target", ".git", "node_modules", "__pycache__", ".girder"]
        .iter()
        .flat_map(|name| {
            [
                name.to_string(),
                format!("**/{name}"),
                format!("**/{name}/**"),
            ]
        })
        .collect()
}

impl Snapshot {
    pub fn capture(root: &Path, config: &ProjectConfig) -> io::Result<Self> {
        let metadata = std::fs::metadata(root)?;
        if !metadata.is_dir() {
            return Err(io::Error::other("watched root is not a directory"));
        }
        let mut files = BTreeMap::new();
        for (absolute, relative) in collect_sources_with_config(root, config)? {
            let value = stamp(&absolute)?
                .ok_or_else(|| io::Error::other("unstable read: source disappeared"))?;
            files.insert(relative, Some(value));
        }
        let source_paths = files.keys().cloned().collect();
        for name in ["girder.toml", "Cargo.toml", "go.mod"] {
            files.insert(name.to_string(), stamp(&root.join(name))?);
        }
        Ok(Self {
            files,
            source_paths,
            root_created: metadata.created().ok(),
            #[cfg(unix)]
            root_identity: {
                use std::os::unix::fs::MetadataExt;
                (metadata.dev(), metadata.ino())
            },
            graph: read_project_bytes(root, &config.graph.path)?,
        })
    }

    pub fn sources_equal(&self, other: &Self) -> bool {
        self.files == other.files && self.root_created == other.root_created && {
            #[cfg(unix)]
            {
                self.root_identity == other.root_identity
            }
            #[cfg(not(unix))]
            {
                true
            }
        }
    }

    pub fn matches_candidate(&self, project: &crate::project::source::CachedProject) -> bool {
        let paths: std::collections::BTreeSet<_> =
            project.builder.source_files().into_iter().collect();
        paths == self.source_paths
            && paths.iter().all(|p| {
                project
                    .builder
                    .source_of(p)
                    .is_some_and(|source| self.matches_bytes(p, Some(source.as_bytes())))
            })
            && project
                .configuration_inputs()
                .iter()
                .all(|(p, bytes)| self.matches_bytes(p, bytes.as_deref()))
            && self.graph == project.persisted_bytes
    }

    pub(super) fn matches_bytes(&self, path: &str, bytes: Option<&[u8]>) -> bool {
        match (self.files.get(path), bytes) {
            (Some(Some(stamp)), Some(bytes)) => {
                stamp.digest == <[u8; 32]>::from(Sha256::digest(bytes))
            }
            (Some(None), None) => true,
            _ => false,
        }
    }

    pub fn dirty_since(&self, old: &Self) -> Vec<String> {
        self.files
            .keys()
            .chain(old.files.keys())
            .filter(|p| self.files.get(*p) != old.files.get(*p))
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

fn stamp(path: &Path) -> io::Result<Option<Stamp>> {
    let before = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !before.is_file() {
        return Err(io::Error::other(
            "source or configuration input is not a regular file",
        ));
    }
    let bytes = std::fs::read(path)?;
    let after = std::fs::metadata(path)?;
    if before.modified()? != after.modified()?
        || before.len() != after.len()
        || bytes.len() as u64 != after.len()
    {
        return Err(io::Error::other(
            "unstable read: input changed while being read",
        ));
    }
    Ok(Some(Stamp {
        modified: after.modified()?,
        size: after.len(),
        digest: Sha256::digest(&bytes).into(),
    }))
}

pub(super) fn graph_identity(
    root: &Path,
    config: &ProjectConfig,
) -> io::Result<std::path::PathBuf> {
    let path = safe_project_input_path(root, &config.graph.path)?;
    crate::project::source::locks::canonical_identity(&path)
}
