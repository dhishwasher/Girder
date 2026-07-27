//! Authenticated, bounded local discovery for live collaboration hosts.
//!
//! Discovery tickets are deliberately only hints. They advertise a loopback
//! address in a private current-user directory and authenticate the bytes with
//! the collaboration group secret. Joining still performs the full live
//! handshake, roster checks, and convergent durable synchronization.

use super::collaboration_transport::{join, read_secret, LiveSyncReport, PROTOCOL_VERSION};
use aether_graph::{ActorId, GraphError, GraphReplica};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

const DISCOVERY_MAGIC: &str = "BITCODE_LOCAL_PEER";
const DISCOVERY_VERSION: u32 = 1;
const DISCOVERY_MAC_TAG: &[u8] = b"bitcode-local-discovery";
const NONCE_BYTES: usize = 32;
const MAC_BYTES: usize = 32;
const MAX_DISCOVERY_FILE_BYTES: usize = 16 * 1024;
const MAX_DISCOVERY_ENTRIES: usize = 256;
const DISCOVERY_EXTENSION: &str = "bitcode-peer";

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscoveredPeer {
    pub(crate) actor: ActorId,
    pub(crate) address: SocketAddr,
    pub(crate) process_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscoveryScan {
    pub(crate) peers: Vec<DiscoveredPeer>,
    pub(crate) ignored_entries: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiscoveryPayload {
    magic: String,
    version: u32,
    live_protocol: u32,
    actor: ActorId,
    address: String,
    process_id: u32,
    instance_nonce: [u8; NONCE_BYTES],
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedDiscoveryTicket {
    payload: DiscoveryPayload,
    mac: [u8; MAC_BYTES],
}

pub(super) struct DiscoveryLease {
    path: PathBuf,
    published_bytes: Vec<u8>,
}

impl DiscoveryLease {
    pub(super) fn publish(
        directory: &Path,
        actor: &ActorId,
        address: SocketAddr,
        secret: &[u8],
    ) -> std::io::Result<Self> {
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err(invalid_input(
                "discovery tickets require a concrete loopback address",
            ));
        }
        prepare_discovery_directory(directory)?;
        let instance_nonce = random_nonce()?;
        let payload = DiscoveryPayload {
            magic: DISCOVERY_MAGIC.into(),
            version: DISCOVERY_VERSION,
            live_protocol: PROTOCOL_VERSION,
            actor: actor.clone(),
            address: address.to_string(),
            process_id: std::process::id(),
            instance_nonce,
        };
        let ticket = SignedDiscoveryTicket {
            mac: ticket_mac(secret, &payload)?,
            payload,
        };
        let mut published_bytes =
            serde_json::to_vec(&ticket).map_err(|error| invalid_data(error.to_string()))?;
        published_bytes.push(b'\n');
        if published_bytes.len() > MAX_DISCOVERY_FILE_BYTES {
            return Err(invalid_data(
                "local discovery ticket exceeds its size limit",
            ));
        }

        let file_name = ticket_file_name(actor, &instance_nonce);
        let path = directory.join(file_name);
        publish_complete_file(&path, &published_bytes)?;
        Ok(Self {
            path,
            published_bytes,
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for DiscoveryLease {
    fn drop(&mut self) {
        let unchanged = read_private_file(&self.path, MAX_DISCOVERY_FILE_BYTES)
            .map(|bytes| bytes == self.published_bytes)
            .unwrap_or(false);
        if unchanged && std::fs::remove_file(&self.path).is_ok() {
            if let Some(parent) = self.path.parent() {
                let _ = std::fs::File::open(parent).and_then(|directory| directory.sync_all());
            }
        }
    }
}

pub(crate) fn discover(
    bundle: &Path,
    directory: &Path,
    secret_file: &Path,
) -> std::io::Result<DiscoveryScan> {
    validate_discovery_directory(directory)?;
    let secret = read_secret(secret_file)?;
    let replica = collaboration_result(GraphReplica::load(bundle))?;
    let mut paths = Vec::new();
    let mut ignored_entries = 0;
    let mut entry_count = 0;
    for entry in std::fs::read_dir(directory)? {
        entry_count += 1;
        if entry_count > MAX_DISCOVERY_ENTRIES {
            return Err(invalid_data(format!(
                "local discovery directory exceeds {MAX_DISCOVERY_ENTRIES} entries"
            )));
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                ignored_entries += 1;
                continue;
            }
        };
        paths.push(entry.path());
    }
    paths.sort();

    let mut peers = Vec::new();
    for path in paths {
        if path.extension().and_then(|extension| extension.to_str()) != Some(DISCOVERY_EXTENSION) {
            ignored_entries += 1;
            continue;
        }
        match read_peer(&path, &secret, &replica) {
            Ok(peer) => peers.push(peer),
            Err(_) => ignored_entries += 1,
        }
    }
    peers.sort_by(|left, right| {
        left.actor
            .cmp(&right.actor)
            .then_with(|| left.address.cmp(&right.address))
            .then_with(|| left.process_id.cmp(&right.process_id))
    });
    Ok(DiscoveryScan {
        peers,
        ignored_entries,
    })
}

pub(crate) fn join_peer(
    bundle: &Path,
    actor: &str,
    directory: &Path,
    secret_file: &Path,
    out: Option<&Path>,
    presence: Option<&str>,
) -> std::io::Result<LiveSyncReport> {
    let actor = collaboration_result(ActorId::new(actor))?;
    let scan = discover(bundle, directory, secret_file)?;
    let matching = scan
        .peers
        .iter()
        .filter(|peer| peer.actor == actor)
        .collect::<Vec<_>>();
    let [peer] = matching.as_slice() else {
        return if matching.is_empty() {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no active local discovery ticket for actor '{actor}'"),
            ))
        } else {
            Err(invalid_input(format!(
                "multiple active local discovery tickets for actor '{actor}'; use an explicit address"
            )))
        };
    };
    join(
        bundle,
        &peer.address.to_string(),
        secret_file,
        out,
        presence,
    )
}

fn read_peer(
    path: &Path,
    secret: &[u8],
    replica: &GraphReplica,
) -> std::io::Result<DiscoveredPeer> {
    let bytes = read_private_file(path, MAX_DISCOVERY_FILE_BYTES)?;
    let ticket: SignedDiscoveryTicket =
        serde_json::from_slice(&bytes).map_err(|error| invalid_data(error.to_string()))?;
    verify_ticket_mac(secret, &ticket.payload, &ticket.mac)?;
    if ticket.payload.magic != DISCOVERY_MAGIC
        || ticket.payload.version != DISCOVERY_VERSION
        || ticket.payload.live_protocol != PROTOCOL_VERSION
    {
        return Err(invalid_data(
            "unsupported local collaboration discovery ticket",
        ));
    }
    let actor = collaboration_result(ActorId::new(ticket.payload.actor.as_str()))?;
    let address: SocketAddr = ticket
        .payload
        .address
        .parse()
        .map_err(|error| invalid_data(format!("invalid discovery address: {error}")))?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err(invalid_data(
            "local discovery ticket contains a non-loopback address",
        ));
    }
    if ticket.payload.process_id == 0 || !process_is_alive(ticket.payload.process_id) {
        return Err(invalid_data("local discovery ticket owner is not running"));
    }
    let expected_name = ticket_file_name(&actor, &ticket.payload.instance_nonce);
    if path.file_name().and_then(|name| name.to_str()) != Some(expected_name.as_str()) {
        return Err(invalid_data(
            "local discovery ticket name does not match its authenticated payload",
        ));
    }
    if actor == *replica.actor() || !collaboration_result(replica.is_member(&actor))? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("discovered actor '{actor}' is not an active remote member"),
        ));
    }
    Ok(DiscoveredPeer {
        actor,
        address,
        process_id: ticket.payload.process_id,
    })
}

fn ticket_file_name(actor: &ActorId, nonce: &[u8; NONCE_BYTES]) -> String {
    let nonce = nonce
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{actor}-{nonce}.{DISCOVERY_EXTENSION}")
}

fn ticket_mac(secret: &[u8], payload: &DiscoveryPayload) -> std::io::Result<[u8; MAC_BYTES]> {
    let bytes = serde_json::to_vec(payload).map_err(|error| invalid_data(error.to_string()))?;
    let mut mac = HmacSha256::new_from_slice(secret)
        .map_err(|_| invalid_input("invalid collaboration secret"))?;
    mac.update(DISCOVERY_MAC_TAG);
    mac.update(&bytes);
    Ok(mac.finalize().into_bytes().into())
}

fn verify_ticket_mac(
    secret: &[u8],
    payload: &DiscoveryPayload,
    expected: &[u8; MAC_BYTES],
) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(payload).map_err(|error| invalid_data(error.to_string()))?;
    let mut mac = HmacSha256::new_from_slice(secret)
        .map_err(|_| invalid_input("invalid collaboration secret"))?;
    mac.update(DISCOVERY_MAC_TAG);
    mac.update(&bytes);
    mac.verify_slice(expected).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "invalid local discovery ticket MAC",
        )
    })
}

fn random_nonce() -> std::io::Result<[u8; NONCE_BYTES]> {
    let mut nonce = [0; NONCE_BYTES];
    getrandom::fill(&mut nonce).map_err(|error| std::io::Error::other(error.to_string()))?;
    Ok(nonce)
}

fn prepare_discovery_directory(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(path)?;
        }
        Err(error) => return Err(error),
    }
    validate_discovery_directory(path)
}

fn validate_discovery_directory(path: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid_input(format!(
            "local discovery path must be a real directory: {}",
            path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "local discovery directory must be owned by the current user",
            ));
        }
        if metadata.mode() & 0o077 != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "local discovery directory must not be accessible by group/others (use chmod 700)",
            ));
        }
    }
    Ok(())
}

fn publish_complete_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| invalid_input("local discovery ticket needs a parent directory"))?;
    let temporary = parent.join(format!(
        ".bitcode-peer-{}-{}.tmp",
        std::process::id(),
        random_nonce()?
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut temporary_created = false;
    let mut published = false;
    let result = (|| {
        let mut file = options.open(&temporary)?;
        temporary_created = true;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::hard_link(&temporary, path)?;
        published = true;
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if temporary_created {
        let _ = std::fs::remove_file(&temporary);
    }
    if result.is_err() && published {
        let _ = std::fs::remove_file(path);
    }
    result
}

fn read_private_file(path: &Path, limit: usize) -> std::io::Result<Vec<u8>> {
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
                "local discovery ticket must not be a symlink: {}",
                path.display()
            )));
        }
        std::fs::File::open(path)?
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() as usize > limit {
        return Err(invalid_data(format!(
            "local discovery ticket is not a regular file within {limit} bytes"
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "local discovery ticket must be current-user-owned and mode 600",
            ));
        }
    }
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > limit {
        return Err(invalid_data(format!(
            "local discovery ticket must contain 1..={limit} bytes"
        )));
    }
    Ok(bytes)
}

#[cfg(unix)]
fn process_is_alive(process_id: u32) -> bool {
    let Ok(process_id) = i32::try_from(process_id) else {
        return false;
    };
    let result = unsafe { libc::kill(process_id, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn process_is_alive(_process_id: u32) -> bool {
    true
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

#[cfg(test)]
mod tests {
    use super::*;
    use aether_graph::SemanticGraph;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    const SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "bitcode-local-discovery-{}-{}-{}",
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

    fn write_secret(path: &Path, secret: &[u8]) {
        std::fs::write(path, secret).unwrap();
        set_mode(path, 0o600);
    }

    fn local_bundle(path: &Path) {
        let graph = SemanticGraph::new();
        let mut alice = GraphReplica::from_graph(actor("alice"), &graph);
        let bob = alice.fork(actor("bob")).unwrap();
        bob.save(path).unwrap();
    }

    fn fixture(temp: &TempDir) -> (PathBuf, PathBuf, PathBuf) {
        let bundle = temp.path("bob.aetherc");
        let secret = temp.path("secret");
        let directory = temp.path("peers");
        local_bundle(&bundle);
        write_secret(&secret, SECRET);
        prepare_discovery_directory(&directory).unwrap();
        (bundle, secret, directory)
    }

    fn write_ticket(
        directory: &Path,
        actor_name: &str,
        address: &str,
        process_id: u32,
        nonce_byte: u8,
        secret: &[u8],
    ) -> PathBuf {
        let payload = DiscoveryPayload {
            magic: DISCOVERY_MAGIC.into(),
            version: DISCOVERY_VERSION,
            live_protocol: PROTOCOL_VERSION,
            actor: actor(actor_name),
            address: address.into(),
            process_id,
            instance_nonce: [nonce_byte; NONCE_BYTES],
        };
        let ticket = SignedDiscoveryTicket {
            mac: ticket_mac(secret, &payload).unwrap(),
            payload,
        };
        let path = directory.join(ticket_file_name(
            &ticket.payload.actor,
            &ticket.payload.instance_nonce,
        ));
        let mut bytes = serde_json::to_vec(&ticket).unwrap();
        bytes.push(b'\n');
        std::fs::write(&path, bytes).unwrap();
        set_mode(&path, 0o600);
        path
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(not(unix))]
    fn set_mode(_path: &Path, _mode: u32) {}

    #[test]
    fn published_ticket_is_discoverable_private_and_ephemeral() {
        let temp = TempDir::new();
        let (bundle, secret_file, directory) = fixture(&temp);
        let address = "127.0.0.1:7331".parse().unwrap();
        let lease = DiscoveryLease::publish(&directory, &actor("alice"), address, SECRET).unwrap();

        let bytes = std::fs::read(lease.path()).unwrap();
        assert!(bytes.len() <= MAX_DISCOVERY_FILE_BYTES);
        assert!(!bytes.windows(SECRET.len()).any(|window| window == SECRET));
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(std::fs::metadata(&directory).unwrap().mode() & 0o777, 0o700);
            assert_eq!(
                std::fs::metadata(lease.path()).unwrap().mode() & 0o777,
                0o600
            );
        }

        assert_eq!(
            discover(&bundle, &directory, &secret_file).unwrap(),
            DiscoveryScan {
                peers: vec![DiscoveredPeer {
                    actor: actor("alice"),
                    address,
                    process_id: std::process::id(),
                }],
                ignored_entries: 0,
            }
        );
        let ticket_path = lease.path().to_path_buf();
        drop(lease);
        assert!(!ticket_path.exists());
        assert_eq!(
            discover(&bundle, &directory, &secret_file).unwrap().peers,
            Vec::new()
        );
    }

    #[test]
    fn tampering_or_using_the_wrong_secret_invalidates_a_ticket() {
        let temp = TempDir::new();
        let (bundle, secret_file, directory) = fixture(&temp);
        let lease = DiscoveryLease::publish(
            &directory,
            &actor("alice"),
            "127.0.0.1:7331".parse().unwrap(),
            SECRET,
        )
        .unwrap();
        let wrong_secret = temp.path("wrong-secret");
        write_secret(&wrong_secret, b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let wrong_scan = discover(&bundle, &directory, &wrong_secret).unwrap();
        assert!(wrong_scan.peers.is_empty());
        assert_eq!(wrong_scan.ignored_entries, 1);

        let mut ticket: SignedDiscoveryTicket =
            serde_json::from_slice(&std::fs::read(lease.path()).unwrap()).unwrap();
        ticket.payload.address = "127.0.0.1:7444".into();
        let mut tampered = serde_json::to_vec(&ticket).unwrap();
        tampered.push(b'\n');
        std::fs::write(lease.path(), tampered).unwrap();
        let tampered_scan = discover(&bundle, &directory, &secret_file).unwrap();
        assert!(tampered_scan.peers.is_empty());
        assert_eq!(tampered_scan.ignored_entries, 1);

        let ticket_path = lease.path().to_path_buf();
        drop(lease);
        assert!(
            ticket_path.exists(),
            "a changed ticket must not be deleted by the original lease"
        );
    }

    #[test]
    fn stale_self_and_unlisted_tickets_are_ignored() {
        let temp = TempDir::new();
        let (bundle, secret_file, directory) = fixture(&temp);
        write_ticket(
            &directory,
            "alice",
            "127.0.0.1:7331",
            std::process::id(),
            1,
            SECRET,
        );
        write_ticket(&directory, "alice", "127.0.0.1:7332", u32::MAX, 2, SECRET);
        write_ticket(
            &directory,
            "bob",
            "127.0.0.1:7333",
            std::process::id(),
            3,
            SECRET,
        );
        write_ticket(
            &directory,
            "mallory",
            "127.0.0.1:7334",
            std::process::id(),
            4,
            SECRET,
        );
        write_ticket(
            &directory,
            "alice",
            "192.0.2.1:7335",
            std::process::id(),
            5,
            SECRET,
        );

        let scan = discover(&bundle, &directory, &secret_file).unwrap();
        assert_eq!(scan.peers.len(), 1);
        assert_eq!(scan.peers[0].actor, actor("alice"));
        assert_eq!(scan.ignored_entries, 4);
    }

    #[test]
    fn ticket_name_must_match_the_authenticated_instance() {
        let temp = TempDir::new();
        let (bundle, secret_file, directory) = fixture(&temp);
        let ticket = write_ticket(
            &directory,
            "alice",
            "127.0.0.1:7331",
            std::process::id(),
            1,
            SECRET,
        );
        let copied = directory.join(format!("alice-renamed.{DISCOVERY_EXTENSION}"));
        std::fs::copy(ticket, &copied).unwrap();
        set_mode(&copied, 0o600);

        let scan = discover(&bundle, &directory, &secret_file).unwrap();
        assert_eq!(scan.peers.len(), 1);
        assert_eq!(scan.ignored_entries, 1);
    }

    #[test]
    fn duplicate_actor_tickets_make_join_peer_fail_closed() {
        let temp = TempDir::new();
        let (bundle, secret_file, directory) = fixture(&temp);
        for (port, nonce) in [(7331, 1), (7332, 2)] {
            write_ticket(
                &directory,
                "alice",
                &format!("127.0.0.1:{port}"),
                std::process::id(),
                nonce,
                SECRET,
            );
        }

        let error = join_peer(&bundle, "alice", &directory, &secret_file, None, None).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("multiple active"));
    }

    #[test]
    fn discovery_directory_has_a_hard_entry_limit() {
        let temp = TempDir::new();
        let (bundle, secret_file, directory) = fixture(&temp);
        for index in 0..=MAX_DISCOVERY_ENTRIES {
            std::fs::write(directory.join(format!("entry-{index}")), b"x").unwrap();
        }

        let error = discover(&bundle, &directory, &secret_file).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("exceeds 256 entries"));
    }

    #[test]
    fn publish_rejects_non_loopback_or_unbound_addresses() {
        let temp = TempDir::new();
        let directory = temp.path("peers");
        for address in ["192.0.2.1:7331", "127.0.0.1:0"] {
            assert!(
                DiscoveryLease::publish(
                    &directory,
                    &actor("alice"),
                    address.parse().unwrap(),
                    SECRET
                )
                .is_err(),
                "published unsafe address {address}"
            );
        }
        assert!(!directory.exists());
    }

    #[cfg(unix)]
    #[test]
    fn broad_permissions_and_symlinks_are_rejected() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new();
        let (_bundle, _secret_file, directory) = fixture(&temp);
        let ticket = write_ticket(
            &directory,
            "alice",
            "127.0.0.1:7331",
            std::process::id(),
            1,
            SECRET,
        );
        set_mode(&ticket, 0o640);
        assert_eq!(
            read_private_file(&ticket, MAX_DISCOVERY_FILE_BYTES)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );

        set_mode(&ticket, 0o600);
        let linked = directory.join(format!("linked.{DISCOVERY_EXTENSION}"));
        symlink(&ticket, &linked).unwrap();
        assert!(read_private_file(&linked, MAX_DISCOVERY_FILE_BYTES).is_err());

        let linked_directory = temp.path("linked-peers");
        symlink(&directory, &linked_directory).unwrap();
        assert_eq!(
            validate_discovery_directory(&linked_directory)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidInput
        );

        set_mode(&directory, 0o750);
        assert_eq!(
            validate_discovery_directory(&directory).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }
}
