//! Shared resolver/watcher census of non-source proof inputs.
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// Exact inputs used by the conditional ESM proof. Equality includes contents
/// and entry kind, including empty mock directories and symlink identities.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeScriptEsmEnvironment {
    inputs: BTreeMap<String, Vec<u8>>,
    blocked: bool,
}

fn hazard_name(name: &str) -> bool {
    matches!(name, "__mocks__" | ".npmrc" | ".taprc")
        || [
            "vitest.config.",
            "vite.config.",
            "vitest.workspace.",
            "jest.config.",
            ".mocharc.",
            ".c8rc",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

impl TypeScriptEsmEnvironment {
    /// Conservative event filter. Includes excluded and hidden paths because
    /// the proof's environment scope is the entire root listing.
    pub fn is_input_path(relative: &str) -> bool {
        relative
            .split('/')
            .any(|name| name == "package.json" || hazard_name(name))
    }

    pub fn contains_path_or_descendant(&self, relative: &str) -> bool {
        self.inputs
            .keys()
            .any(|p| p == relative || p.starts_with(&format!("{relative}/")))
    }

    pub fn dirty_since(&self, old: &Self) -> Vec<String> {
        self.inputs
            .keys()
            .chain(old.inputs.keys())
            .filter(|p| self.inputs.get(*p) != old.inputs.get(*p))
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub(super) fn is_clear(&self) -> bool {
        !self.blocked
    }

    /// Do not follow links or omit hidden/excluded configuration. A failed or
    /// unstable read is uncertainty; callers must not use an older census.
    pub fn capture(root: &Path) -> io::Result<Self> {
        let mut result = Self::default();
        let mut pending = vec![root.to_path_buf()];
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                let name = entry.file_name();
                let name = name
                    .to_str()
                    .ok_or_else(|| io::Error::other("non-UTF-8 ESM environment identity"))?;
                let kind = entry.file_type()?;
                if hazard_name(name) || name == "package.json" {
                    let relative = path
                        .strip_prefix(root)
                        .map_err(io::Error::other)?
                        .to_str()
                        .ok_or_else(|| io::Error::other("non-UTF-8 ESM environment path"))?
                        .replace('\\', "/");
                    let (tag, contents) = if kind.is_symlink() {
                        (
                            b'L',
                            std::fs::read_link(&path)?
                                .to_str()
                                .ok_or_else(|| io::Error::other("non-UTF-8 environment symlink"))?
                                .as_bytes()
                                .to_vec(),
                        )
                    } else if kind.is_dir() {
                        (b'D', Vec::new())
                    } else if kind.is_file() {
                        (b'F', stable_bytes(&path)?)
                    } else {
                        (b'O', Vec::new())
                    };
                    result.blocked |= hazard_name(name);
                    if name == "package.json" {
                        // Preserve the resolver's conservative textual rule.
                        // Escapes can hide keys/flags; no partial JSON decoder.
                        result.blocked |= tag != b'F'
                            || std::str::from_utf8(&contents).map_or(true, |text| {
                                text.contains('\\')
                                    || text.contains("\"jest\"")
                                    || [
                                        "--import",
                                        "--require",
                                        "-r ",
                                        "--loader",
                                        "--experimental-loader",
                                        "--experimental-test-module-mocks",
                                        "--experimental-default-type",
                                    ]
                                    .iter()
                                    .any(|flag| text.contains(flag))
                            });
                    }
                    let mut identity = vec![tag];
                    identity.extend(contents);
                    result.inputs.insert(relative, identity);
                }
                if kind.is_dir() {
                    pending.push(path);
                }
            }
        }
        Ok(result)
    }
}

fn stable_bytes(path: &Path) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let before = std::fs::symlink_metadata(path)?;
    if !before.is_file() {
        return Err(io::Error::other("ESM environment input changed kind"));
    }
    let mut file = std::fs::File::open(path)?;
    let opened = file.metadata()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let after = std::fs::symlink_metadata(path)?;
    let stable = after.is_file()
        && before.len() == after.len()
        && bytes.len() as u64 == after.len()
        && before.modified()? == after.modified()?
        && opened.modified()? == after.modified()?;
    #[cfg(unix)]
    let stable = {
        use std::os::unix::fs::MetadataExt;
        stable
            && before.dev() == opened.dev()
            && before.ino() == opened.ino()
            && opened.dev() == after.dev()
            && opened.ino() == after.ino()
    };
    if !stable {
        return Err(io::Error::other("unstable ESM environment input"));
    }
    Ok(bytes)
}
