//! Durable local Ed25519 identities and pinned peer trust for live collaboration.
//!
//! The collaboration group secret remains a bootstrap and session-integrity
//! credential. Identity-pinned sessions additionally prove that each claimed
//! actor controls a locally trusted Ed25519 key, so another group-secret holder
//! cannot impersonate that actor. Public identity records are meant to be
//! fingerprint-verified out of band before they are pinned.

use aether_graph::{ActorId, GraphError, GraphReplica};
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

const PRIVATE_IDENTITY_MAGIC: &str = "BITCODE_ACTOR_IDENTITY_PRIVATE";
const PUBLIC_IDENTITY_MAGIC: &str = "BITCODE_ACTOR_IDENTITY_PUBLIC";
const TRUST_STORE_MAGIC: &str = "BITCODE_ACTOR_TRUST";
const IDENTITY_VERSION: u32 = 1;
const IDENTITY_SIGNATURE_CONTEXT: &[u8] = b"bitcode-live-actor-identity-v1";
pub(crate) const PUBLIC_KEY_BYTES: usize = 32;
const PRIVATE_SEED_BYTES: usize = 32;
const MAX_SIGNATURE_BYTES: usize = 64;
const MAX_IDENTITY_FILE_BYTES: usize = 16 * 1024;
const MAX_TRUST_STORE_BYTES: usize = 256 * 1024;
const MAX_TRUSTED_IDENTITIES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IdentitySummary {
    pub(crate) actor: ActorId,
    pub(crate) fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TrustChange {
    Added(IdentitySummary),
    AlreadyTrusted(IdentitySummary),
    Rotated {
        actor: ActorId,
        previous_fingerprint: String,
        fingerprint: String,
    },
    Removed(IdentitySummary),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivateIdentityFile {
    magic: String,
    version: u32,
    actor: ActorId,
    private_seed: [u8; PRIVATE_SEED_BYTES],
    public_key: [u8; PUBLIC_KEY_BYTES],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicIdentityFile {
    magic: String,
    version: u32,
    actor: ActorId,
    public_key: [u8; PUBLIC_KEY_BYTES],
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustStoreFile {
    magic: String,
    version: u32,
    identities: Vec<PublicIdentityFile>,
}

struct TrustStoreState {
    identities: BTreeMap<ActorId, [u8; PUBLIC_KEY_BYTES]>,
    original_bytes: Option<Vec<u8>>,
}

#[derive(Debug)]
pub(crate) struct SessionIdentity {
    actor: ActorId,
    key_pair: Ed25519KeyPair,
    public_key: [u8; PUBLIC_KEY_BYTES],
    trusted: BTreeMap<ActorId, [u8; PUBLIC_KEY_BYTES]>,
}

impl SessionIdentity {
    pub(crate) fn load(
        identity_file: &Path,
        trust_store: &Path,
        expected_actor: &ActorId,
    ) -> std::io::Result<Self> {
        let bytes = read_bounded_file(identity_file, MAX_IDENTITY_FILE_BYTES, true)?;
        let identity: PrivateIdentityFile =
            serde_json::from_slice(&bytes).map_err(|error| invalid_data(error.to_string()))?;
        validate_private_identity(&identity)?;
        if &identity.actor != expected_actor {
            return Err(invalid_input(format!(
                "identity file belongs to actor '{}', not local actor '{expected_actor}'",
                identity.actor
            )));
        }
        let key_pair =
            Ed25519KeyPair::from_seed_and_public_key(&identity.private_seed, &identity.public_key)
                .map_err(|_| invalid_data("identity private and public keys are inconsistent"))?;
        let trusted = load_trust_store(trust_store)?.identities;
        Ok(Self {
            actor: identity.actor,
            key_pair,
            public_key: identity.public_key,
            trusted,
        })
    }

    pub(crate) fn actor(&self) -> &ActorId {
        &self.actor
    }

    pub(crate) fn public_key(&self) -> &[u8; PUBLIC_KEY_BYTES] {
        &self.public_key
    }

    pub(crate) fn fingerprint(&self) -> String {
        fingerprint(&self.public_key)
    }

    pub(crate) fn sign(&self, role: &[u8], transcript: &[u8]) -> Vec<u8> {
        self.key_pair
            .sign(&identity_signature_message(role, transcript))
            .as_ref()
            .to_vec()
    }

    pub(crate) fn verify_peer(
        &self,
        actor: &ActorId,
        presented_key: &[u8; PUBLIC_KEY_BYTES],
        role: &[u8],
        transcript: &[u8],
        signature: &[u8],
    ) -> std::io::Result<String> {
        if signature.len() != MAX_SIGNATURE_BYTES {
            return Err(permission_denied(format!(
                "invalid identity signature length for actor '{actor}'"
            )));
        }
        let Some(trusted_key) = self.trusted.get(actor) else {
            return Err(permission_denied(format!(
                "no trusted identity is pinned for actor '{actor}'"
            )));
        };
        if trusted_key != presented_key {
            return Err(permission_denied(format!(
                "identity key for actor '{actor}' does not match the local trust store"
            )));
        }
        UnparsedPublicKey::new(&ED25519, presented_key)
            .verify(&identity_signature_message(role, transcript), signature)
            .map_err(|_| {
                permission_denied(format!(
                    "invalid Ed25519 identity signature for actor '{actor}'"
                ))
            })?;
        Ok(fingerprint(presented_key))
    }
}

pub(crate) fn generate_identity(
    bundle: &Path,
    private_path: &Path,
    public_path: &Path,
) -> std::io::Result<IdentitySummary> {
    if paths_alias(private_path, public_path) {
        return Err(invalid_input(
            "private and public identity outputs must be different paths",
        ));
    }
    let replica = collaboration_result(GraphReplica::load(bundle))?;
    if !collaboration_result(replica.is_member(replica.actor()))? {
        return Err(invalid_input(format!(
            "local actor '{}' is not an active collaboration member",
            replica.actor()
        )));
    }
    generate_for_actor(replica.actor(), private_path, public_path)
}

pub(crate) fn inspect_public_identity(path: &Path) -> std::io::Result<IdentitySummary> {
    let identity = read_public_identity(path)?;
    Ok(identity_summary(&identity.actor, &identity.public_key))
}

pub(crate) fn trusted_identities(path: &Path) -> std::io::Result<Vec<IdentitySummary>> {
    Ok(load_trust_store(path)?
        .identities
        .into_iter()
        .map(|(actor, key)| identity_summary(&actor, &key))
        .collect())
}

pub(crate) fn trust_identity(
    trust_store: &Path,
    public_identity: &Path,
    approved_fingerprint: &str,
) -> std::io::Result<TrustChange> {
    let identity = read_public_identity(public_identity)?;
    require_fingerprint_approval(&identity.public_key, approved_fingerprint)?;
    let mut state = load_trust_store_or_empty(trust_store)?;
    let summary = identity_summary(&identity.actor, &identity.public_key);
    match state.identities.get(&identity.actor) {
        Some(existing) if existing == &identity.public_key => {
            Ok(TrustChange::AlreadyTrusted(summary))
        }
        Some(existing) => Err(invalid_input(format!(
            "actor '{}' is already pinned to {}; use identity rotate with that exact old fingerprint",
            identity.actor,
            fingerprint(existing)
        ))),
        None => {
            if state.identities.len() >= MAX_TRUSTED_IDENTITIES {
                return Err(invalid_data(format!(
                    "identity trust store exceeds {MAX_TRUSTED_IDENTITIES} actors"
                )));
            }
            state
                .identities
                .insert(identity.actor, identity.public_key);
            save_trust_store(trust_store, &state)?;
            Ok(TrustChange::Added(summary))
        }
    }
}

pub(crate) fn rotate_identity(
    trust_store: &Path,
    public_identity: &Path,
    expected_old_fingerprint: &str,
    approved_new_fingerprint: &str,
) -> std::io::Result<TrustChange> {
    let identity = read_public_identity(public_identity)?;
    require_fingerprint_approval(&identity.public_key, approved_new_fingerprint)?;
    let mut state = load_trust_store(trust_store)?;
    let Some(previous) = state.identities.get(&identity.actor).copied() else {
        return Err(invalid_input(format!(
            "actor '{}' has no pinned identity to rotate",
            identity.actor
        )));
    };
    require_exact_fingerprint(&previous, expected_old_fingerprint, "old")?;
    if previous == identity.public_key {
        return Err(invalid_input(format!(
            "actor '{}' is already pinned to the proposed identity",
            identity.actor
        )));
    }
    state
        .identities
        .insert(identity.actor.clone(), identity.public_key);
    save_trust_store(trust_store, &state)?;
    Ok(TrustChange::Rotated {
        actor: identity.actor,
        previous_fingerprint: fingerprint(&previous),
        fingerprint: fingerprint(&identity.public_key),
    })
}

pub(crate) fn remove_trusted_identity(
    trust_store: &Path,
    actor: &str,
    approved_fingerprint: &str,
) -> std::io::Result<TrustChange> {
    let actor = collaboration_result(ActorId::new(actor))?;
    let mut state = load_trust_store(trust_store)?;
    let Some(key) = state.identities.get(&actor).copied() else {
        return Err(invalid_input(format!(
            "actor '{actor}' has no pinned identity to remove"
        )));
    };
    require_exact_fingerprint(&key, approved_fingerprint, "current")?;
    state.identities.remove(&actor);
    save_trust_store(trust_store, &state)?;
    Ok(TrustChange::Removed(identity_summary(&actor, &key)))
}

fn generate_for_actor(
    actor: &ActorId,
    private_path: &Path,
    public_path: &Path,
) -> std::io::Result<IdentitySummary> {
    let actor = collaboration_result(ActorId::new(actor.as_str()))?;
    let mut private_seed = [0; PRIVATE_SEED_BYTES];
    getrandom::fill(&mut private_seed).map_err(|error| std::io::Error::other(error.to_string()))?;
    let key_pair = Ed25519KeyPair::from_seed_unchecked(&private_seed)
        .map_err(|_| invalid_data("failed to construct Ed25519 identity"))?;
    let public_key: [u8; PUBLIC_KEY_BYTES] = key_pair
        .public_key()
        .as_ref()
        .try_into()
        .map_err(|_| invalid_data("unexpected Ed25519 public-key length"))?;
    let private = PrivateIdentityFile {
        magic: PRIVATE_IDENTITY_MAGIC.into(),
        version: IDENTITY_VERSION,
        actor: actor.clone(),
        private_seed,
        public_key,
    };
    let public = PublicIdentityFile {
        magic: PUBLIC_IDENTITY_MAGIC.into(),
        version: IDENTITY_VERSION,
        actor: actor.clone(),
        public_key,
    };
    let private_bytes = serialized_file(&private, MAX_IDENTITY_FILE_BYTES)?;
    let public_bytes = serialized_file(&public, MAX_IDENTITY_FILE_BYTES)?;
    write_identity_pair(private_path, &private_bytes, public_path, &public_bytes)?;
    Ok(identity_summary(&actor, &public_key))
}

fn read_public_identity(path: &Path) -> std::io::Result<PublicIdentityFile> {
    let bytes = read_bounded_file(path, MAX_IDENTITY_FILE_BYTES, false)?;
    let identity: PublicIdentityFile =
        serde_json::from_slice(&bytes).map_err(|error| invalid_data(error.to_string()))?;
    validate_public_identity(&identity)?;
    Ok(identity)
}

fn validate_private_identity(identity: &PrivateIdentityFile) -> std::io::Result<()> {
    if identity.magic != PRIVATE_IDENTITY_MAGIC || identity.version != IDENTITY_VERSION {
        return Err(invalid_data("unsupported private actor identity file"));
    }
    collaboration_result(ActorId::new(identity.actor.as_str()))?;
    Ed25519KeyPair::from_seed_and_public_key(&identity.private_seed, &identity.public_key)
        .map_err(|_| invalid_data("identity private and public keys are inconsistent"))?;
    Ok(())
}

fn validate_public_identity(identity: &PublicIdentityFile) -> std::io::Result<()> {
    if identity.magic != PUBLIC_IDENTITY_MAGIC || identity.version != IDENTITY_VERSION {
        return Err(invalid_data("unsupported public actor identity file"));
    }
    collaboration_result(ActorId::new(identity.actor.as_str()))?;
    Ok(())
}

fn load_trust_store(path: &Path) -> std::io::Result<TrustStoreState> {
    let bytes = read_bounded_file(path, MAX_TRUST_STORE_BYTES, true)?;
    decode_trust_store(bytes)
}

fn load_trust_store_or_empty(path: &Path) -> std::io::Result<TrustStoreState> {
    match load_trust_store(path) {
        Ok(state) => Ok(state),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(TrustStoreState {
            identities: BTreeMap::new(),
            original_bytes: None,
        }),
        Err(error) => Err(error),
    }
}

fn decode_trust_store(bytes: Vec<u8>) -> std::io::Result<TrustStoreState> {
    let file: TrustStoreFile =
        serde_json::from_slice(&bytes).map_err(|error| invalid_data(error.to_string()))?;
    if file.magic != TRUST_STORE_MAGIC || file.version != IDENTITY_VERSION {
        return Err(invalid_data("unsupported actor identity trust store"));
    }
    if file.identities.len() > MAX_TRUSTED_IDENTITIES {
        return Err(invalid_data(format!(
            "identity trust store exceeds {MAX_TRUSTED_IDENTITIES} actors"
        )));
    }
    let mut identities = BTreeMap::new();
    let mut previous = None;
    for identity in file.identities {
        validate_public_identity(&identity)?;
        if previous
            .as_ref()
            .is_some_and(|actor| actor >= &identity.actor)
        {
            return Err(invalid_data(
                "identity trust store actors must be unique and strictly sorted",
            ));
        }
        previous = Some(identity.actor.clone());
        identities.insert(identity.actor, identity.public_key);
    }
    Ok(TrustStoreState {
        identities,
        original_bytes: Some(bytes),
    })
}

fn save_trust_store(path: &Path, state: &TrustStoreState) -> std::io::Result<()> {
    let file = TrustStoreFile {
        magic: TRUST_STORE_MAGIC.into(),
        version: IDENTITY_VERSION,
        identities: state
            .identities
            .iter()
            .map(|(actor, public_key)| PublicIdentityFile {
                magic: PUBLIC_IDENTITY_MAGIC.into(),
                version: IDENTITY_VERSION,
                actor: actor.clone(),
                public_key: *public_key,
            })
            .collect(),
    };
    let bytes = serialized_file(&file, MAX_TRUST_STORE_BYTES)?;
    match &state.original_bytes {
        Some(original) => atomic_replace_private(path, original, &bytes),
        None => write_new_file(path, &bytes, 0o600),
    }
}

fn serialized_file<T: Serialize>(value: &T, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut bytes =
        serde_json::to_vec_pretty(value).map_err(|error| invalid_data(error.to_string()))?;
    bytes.push(b'\n');
    if bytes.len() > limit {
        return Err(invalid_data(format!("identity file exceeds {limit} bytes")));
    }
    Ok(bytes)
}

fn write_identity_pair(
    private_path: &Path,
    private_bytes: &[u8],
    public_path: &Path,
    public_bytes: &[u8],
) -> std::io::Result<()> {
    write_new_file(private_path, private_bytes, 0o600)?;
    if let Err(error) = write_new_file(public_path, public_bytes, 0o644) {
        if std::fs::remove_file(private_path).is_ok() {
            sync_parent(private_path);
        }
        return Err(error);
    }
    Ok(())
}

fn write_new_file(path: &Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    let parent = parent_directory(path)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut created = false;
    let result = (|| {
        let mut file = options.open(path)?;
        created = true;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() && created {
        let _ = std::fs::remove_file(path);
    }
    result
}

fn atomic_replace_private(path: &Path, original: &[u8], bytes: &[u8]) -> std::io::Result<()> {
    if read_bounded_file(path, MAX_TRUST_STORE_BYTES, true)? != original {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "identity trust store changed since it was read",
        ));
    }
    let parent = parent_directory(path)?;
    let temporary = parent.join(format!(
        ".bitcode-identity-{}-{}.tmp",
        std::process::id(),
        random_suffix()?
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut created = false;
    let result = (|| {
        let mut file = options.open(&temporary)?;
        created = true;
        file.write_all(bytes)?;
        file.sync_all()?;
        if read_bounded_file(path, MAX_TRUST_STORE_BYTES, true)? != original {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "identity trust store changed during update",
            ));
        }
        std::fs::rename(&temporary, path)?;
        created = false;
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if created {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn read_bounded_file(path: &Path, limit: usize, require_private: bool) -> std::io::Result<Vec<u8>> {
    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)?
    };
    #[cfg(not(unix))]
    let file = {
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            return Err(invalid_input(format!(
                "identity file must not be a symlink: {}",
                path.display()
            )));
        }
        std::fs::File::open(path)?
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() as usize > limit {
        return Err(invalid_data(format!(
            "identity path must be a regular file within {limit} bytes"
        )));
    }
    #[cfg(unix)]
    if require_private {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(permission_denied(
                "private identity files must be owned by the current user",
            ));
        }
        if metadata.mode() & 0o077 != 0 {
            return Err(permission_denied(
                "private identity files must not be accessible by group/others (use chmod 600)",
            ));
        }
    }
    #[cfg(not(unix))]
    let _ = require_private;
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > limit {
        return Err(invalid_data(format!(
            "identity file must contain 1..={limit} bytes"
        )));
    }
    Ok(bytes)
}

fn identity_signature_message(role: &[u8], transcript: &[u8]) -> Vec<u8> {
    let mut message =
        Vec::with_capacity(IDENTITY_SIGNATURE_CONTEXT.len() + role.len() + transcript.len() + 16);
    message.extend_from_slice(IDENTITY_SIGNATURE_CONTEXT);
    message.extend_from_slice(&(role.len() as u64).to_be_bytes());
    message.extend_from_slice(role);
    message.extend_from_slice(&(transcript.len() as u64).to_be_bytes());
    message.extend_from_slice(transcript);
    message
}

fn identity_summary(actor: &ActorId, public_key: &[u8; PUBLIC_KEY_BYTES]) -> IdentitySummary {
    IdentitySummary {
        actor: actor.clone(),
        fingerprint: fingerprint(public_key),
    }
}

fn fingerprint(public_key: &[u8; PUBLIC_KEY_BYTES]) -> String {
    Sha256::digest(public_key)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn require_fingerprint_approval(
    key: &[u8; PUBLIC_KEY_BYTES],
    approved: &str,
) -> std::io::Result<()> {
    require_exact_fingerprint(key, approved, "proposed")
}

fn require_exact_fingerprint(
    key: &[u8; PUBLIC_KEY_BYTES],
    supplied: &str,
    label: &str,
) -> std::io::Result<()> {
    let expected = fingerprint(key);
    if supplied != expected {
        return Err(invalid_input(format!(
            "{label} identity fingerprint approval does not match; expected {expected}"
        )));
    }
    Ok(())
}

fn random_suffix() -> std::io::Result<String> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|error| std::io::Error::other(error.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn parent_directory(path: &Path) -> std::io::Result<&Path> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| invalid_input("identity file needs a parent directory"))
}

fn sync_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::File::open(parent).and_then(|directory| directory.sync_all());
    }
}

fn paths_alias(left: &Path, right: &Path) -> bool {
    left == right
        || match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
            (Ok(left), Ok(right)) => left == right,
            _ => false,
        }
}

fn collaboration_result<T>(result: Result<T, GraphError>) -> std::io::Result<T> {
    result.map_err(std::io::Error::other)
}

fn invalid_input(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into())
}

fn invalid_data(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}

fn permission_denied(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::PermissionDenied, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_graph::SemanticGraph;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "bitcode-identity-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn actor(value: &str) -> ActorId {
        ActorId::new(value).unwrap()
    }

    fn bundle(path: &Path, local_actor: &str) {
        GraphReplica::from_graph(actor(local_actor), &SemanticGraph::new())
            .save(path)
            .unwrap();
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(not(unix))]
    fn set_mode(_path: &Path, _mode: u32) {}

    #[test]
    fn generated_identity_is_actor_bound_private_and_never_overwritten() {
        let temp = TempDir::new();
        let bundle_path = temp.path("alice.aetherc");
        let private = temp.path("alice.identity");
        let public = temp.path("alice.identity.pub");
        bundle(&bundle_path, "alice");

        let summary = generate_identity(&bundle_path, &private, &public).unwrap();
        assert_eq!(summary.actor, actor("alice"));
        assert_eq!(summary.fingerprint.len(), 64);
        assert_eq!(summary, inspect_public_identity(&public).unwrap());
        let private_json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&private).unwrap()).unwrap();
        let public_json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&public).unwrap()).unwrap();
        assert!(private_json.get("private_seed").is_some());
        assert!(public_json.get("private_seed").is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(std::fs::metadata(&private).unwrap().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(&public).unwrap().mode() & 0o777, 0o644);
        }

        let before_private = std::fs::read(&private).unwrap();
        let before_public = std::fs::read(&public).unwrap();
        assert!(generate_identity(&bundle_path, &private, &public).is_err());
        assert_eq!(std::fs::read(&private).unwrap(), before_private);
        assert_eq!(std::fs::read(&public).unwrap(), before_public);

        let blocked_private = temp.path("blocked.identity");
        let blocked_public = temp.path("blocked.identity.pub");
        std::fs::write(&blocked_public, b"existing").unwrap();
        assert!(generate_identity(&bundle_path, &blocked_private, &blocked_public).is_err());
        assert!(!blocked_private.exists());
        assert_eq!(std::fs::read(&blocked_public).unwrap(), b"existing");

        let bob_bundle = temp.path("bob.aetherc");
        bundle(&bob_bundle, "bob");
        assert!(SessionIdentity::load(&private, &temp.path("missing"), &actor("bob")).is_err());
    }

    #[test]
    fn trust_changes_require_exact_fingerprints_and_support_rotation_and_removal() {
        let temp = TempDir::new();
        let alice_private = temp.path("alice.identity");
        let alice_public = temp.path("alice.pub");
        let bob_private = temp.path("bob.identity");
        let bob_public = temp.path("bob.pub");
        let bob_rotated_private = temp.path("bob-rotated.identity");
        let bob_rotated_public = temp.path("bob-rotated.pub");
        let trust_store = temp.path("alice.trust");
        generate_for_actor(&actor("alice"), &alice_private, &alice_public).unwrap();
        let bob = generate_for_actor(&actor("bob"), &bob_private, &bob_public).unwrap();
        let rotated =
            generate_for_actor(&actor("bob"), &bob_rotated_private, &bob_rotated_public).unwrap();

        assert!(trust_identity(&trust_store, &bob_public, "wrong").is_err());
        assert!(!trust_store.exists());
        assert_eq!(
            trust_identity(&trust_store, &bob_public, &bob.fingerprint).unwrap(),
            TrustChange::Added(bob.clone())
        );
        assert_eq!(
            trust_identity(&trust_store, &bob_public, &bob.fingerprint).unwrap(),
            TrustChange::AlreadyTrusted(bob.clone())
        );
        assert_eq!(trusted_identities(&trust_store).unwrap(), vec![bob.clone()]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                std::fs::metadata(&trust_store).unwrap().mode() & 0o777,
                0o600
            );
        }

        let error =
            trust_identity(&trust_store, &bob_rotated_public, &rotated.fingerprint).unwrap_err();
        assert!(error.to_string().contains("identity rotate"));
        assert!(rotate_identity(
            &trust_store,
            &bob_rotated_public,
            "wrong-old",
            &rotated.fingerprint
        )
        .is_err());
        assert!(rotate_identity(
            &trust_store,
            &bob_rotated_public,
            &bob.fingerprint,
            "wrong-new"
        )
        .is_err());
        let change = rotate_identity(
            &trust_store,
            &bob_rotated_public,
            &bob.fingerprint,
            &rotated.fingerprint,
        )
        .unwrap();
        assert_eq!(
            change,
            TrustChange::Rotated {
                actor: actor("bob"),
                previous_fingerprint: bob.fingerprint.clone(),
                fingerprint: rotated.fingerprint.clone(),
            }
        );
        assert_eq!(
            trusted_identities(&trust_store).unwrap(),
            vec![rotated.clone()]
        );

        assert!(remove_trusted_identity(&trust_store, "bob", &bob.fingerprint).is_err());
        assert_eq!(
            remove_trusted_identity(&trust_store, "bob", &rotated.fingerprint).unwrap(),
            TrustChange::Removed(rotated)
        );
        assert!(trusted_identities(&trust_store).unwrap().is_empty());
    }

    #[test]
    fn signatures_require_the_pinned_actor_key_and_exact_transcript() {
        let temp = TempDir::new();
        let alice_private = temp.path("alice.identity");
        let alice_public = temp.path("alice.pub");
        let bob_private = temp.path("bob.identity");
        let bob_public = temp.path("bob.pub");
        let attacker_private = temp.path("attacker.identity");
        let attacker_public = temp.path("attacker.pub");
        let alice_trust = temp.path("alice.trust");
        let bob_trust = temp.path("bob.trust");
        let alice = generate_for_actor(&actor("alice"), &alice_private, &alice_public).unwrap();
        let bob = generate_for_actor(&actor("bob"), &bob_private, &bob_public).unwrap();
        let attacker =
            generate_for_actor(&actor("alice"), &attacker_private, &attacker_public).unwrap();
        trust_identity(&alice_trust, &bob_public, &bob.fingerprint).unwrap();
        trust_identity(&bob_trust, &alice_public, &alice.fingerprint).unwrap();
        let alice_session =
            SessionIdentity::load(&alice_private, &alice_trust, &actor("alice")).unwrap();
        let bob_session = SessionIdentity::load(&bob_private, &bob_trust, &actor("bob")).unwrap();
        let attacker_session =
            SessionIdentity::load(&attacker_private, &bob_trust, &actor("alice")).unwrap();
        let transcript = b"bounded authenticated transcript";

        let signature = alice_session.sign(b"server", transcript);
        assert_eq!(
            bob_session
                .verify_peer(
                    &actor("alice"),
                    alice_session.public_key(),
                    b"server",
                    transcript,
                    &signature,
                )
                .unwrap(),
            alice.fingerprint
        );
        assert!(bob_session
            .verify_peer(
                &actor("alice"),
                alice_session.public_key(),
                b"client",
                transcript,
                &signature,
            )
            .is_err());
        assert!(bob_session
            .verify_peer(
                &actor("alice"),
                alice_session.public_key(),
                b"server",
                b"altered transcript",
                &signature,
            )
            .is_err());

        let attacker_signature = attacker_session.sign(b"server", transcript);
        let error = bob_session
            .verify_peer(
                &actor("alice"),
                attacker_session.public_key(),
                b"server",
                transcript,
                &attacker_signature,
            )
            .unwrap_err();
        assert!(error.to_string().contains("does not match"));
        assert_ne!(attacker.fingerprint, alice.fingerprint);
    }

    #[cfg(unix)]
    #[test]
    fn private_files_reject_broad_permissions_symlinks_and_stale_updates() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new();
        let alice_private = temp.path("alice.identity");
        let alice_public = temp.path("alice.pub");
        let bob_private = temp.path("bob.identity");
        let bob_public = temp.path("bob.pub");
        let trust_store = temp.path("alice.trust");
        let bob = generate_for_actor(&actor("bob"), &bob_private, &bob_public).unwrap();
        generate_for_actor(&actor("alice"), &alice_private, &alice_public).unwrap();
        trust_identity(&trust_store, &bob_public, &bob.fingerprint).unwrap();

        set_mode(&alice_private, 0o640);
        assert_eq!(
            SessionIdentity::load(&alice_private, &trust_store, &actor("alice"))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
        set_mode(&alice_private, 0o600);
        let linked = temp.path("linked.identity");
        symlink(&alice_private, &linked).unwrap();
        assert!(SessionIdentity::load(&linked, &trust_store, &actor("alice")).is_err());

        let state = load_trust_store(&trust_store).unwrap();
        let external = b"{\"external\":true}\n";
        std::fs::write(&trust_store, external).unwrap();
        set_mode(&trust_store, 0o600);
        assert_eq!(
            save_trust_store(&trust_store, &state).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(std::fs::read(&trust_store).unwrap(), external);
    }

    #[test]
    fn malformed_or_oversized_trust_stores_are_rejected() {
        let temp = TempDir::new();
        let duplicate_path = temp.path("duplicate.trust");
        let key = [7; PUBLIC_KEY_BYTES];
        let duplicate = TrustStoreFile {
            magic: TRUST_STORE_MAGIC.into(),
            version: IDENTITY_VERSION,
            identities: vec![
                PublicIdentityFile {
                    magic: PUBLIC_IDENTITY_MAGIC.into(),
                    version: IDENTITY_VERSION,
                    actor: actor("alice"),
                    public_key: key,
                },
                PublicIdentityFile {
                    magic: PUBLIC_IDENTITY_MAGIC.into(),
                    version: IDENTITY_VERSION,
                    actor: actor("alice"),
                    public_key: key,
                },
            ],
        };
        std::fs::write(
            &duplicate_path,
            serialized_file(&duplicate, 100_000).unwrap(),
        )
        .unwrap();
        set_mode(&duplicate_path, 0o600);
        assert!(trusted_identities(&duplicate_path).is_err());

        let oversized_path = temp.path("oversized.trust");
        let identities = (0..=MAX_TRUSTED_IDENTITIES)
            .map(|index| PublicIdentityFile {
                magic: PUBLIC_IDENTITY_MAGIC.into(),
                version: IDENTITY_VERSION,
                actor: actor(&format!("actor-{index:03}")),
                public_key: [index as u8; PUBLIC_KEY_BYTES],
            })
            .collect();
        let oversized = TrustStoreFile {
            magic: TRUST_STORE_MAGIC.into(),
            version: IDENTITY_VERSION,
            identities,
        };
        std::fs::write(
            &oversized_path,
            serialized_file(&oversized, MAX_TRUST_STORE_BYTES).unwrap(),
        )
        .unwrap();
        set_mode(&oversized_path, 0o600);
        let error = trusted_identities(&oversized_path).unwrap_err();
        assert!(error.to_string().contains("exceeds 256 actors"));
    }
}
