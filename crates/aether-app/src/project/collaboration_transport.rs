//! Authenticated, bounded live transport for graph collaboration replicas.
//!
//! The protocol deliberately binds only loopback sockets. Remote peers should
//! use an authenticated encrypted tunnel (for example SSH) because the session
//! provides mutual authentication and integrity, but not confidentiality.

use aether_graph::{ActorId, GraphDelta, GraphError, GraphReplica, MergeReport, VersionVector};
use hmac::{Hmac, Mac};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::Sha256;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;

const PROTOCOL_MAGIC: &str = "BITCODE_LIVE_COLLAB";
const PROTOCOL_VERSION: u32 = 2;
const NONCE_BYTES: usize = 32;
const MAC_BYTES: usize = 32;
const MIN_SECRET_BYTES: usize = 32;
const MAX_SECRET_BYTES: usize = 4096;
const MAX_HANDSHAKE_BYTES: usize = 64 * 1024;
const MAX_SYNC_BYTES: usize = 16 * 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(15);

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiveSyncReport {
    pub(crate) peer: ActorId,
    pub(crate) sent_operations: usize,
    pub(crate) received_operations: usize,
    pub(crate) inserted_operations: usize,
    pub(crate) node_count: usize,
    pub(crate) edge_count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct Challenge {
    magic: String,
    version: u32,
    server_actor: ActorId,
    server_nonce: [u8; NONCE_BYTES],
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthRequest {
    client_actor: ActorId,
    client_nonce: [u8; NONCE_BYTES],
    client_version: VersionVector,
    proof: [u8; MAC_BYTES],
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthResponse {
    accepted: bool,
    server_actor: ActorId,
    proof: [u8; MAC_BYTES],
    error: Option<String>,
}

#[derive(Serialize)]
struct AuthTranscript<'a> {
    magic: &'a str,
    version: u32,
    server_actor: &'a ActorId,
    client_actor: &'a ActorId,
    server_nonce: &'a [u8; NONCE_BYTES],
    client_nonce: &'a [u8; NONCE_BYTES],
    client_version: &'a VersionVector,
}

#[derive(Debug, Serialize, Deserialize)]
enum SyncPayload {
    ServerDelta {
        delta: GraphDelta,
        version: VersionVector,
    },
    ClientDelta {
        delta: GraphDelta,
    },
    Ack {
        version: VersionVector,
        report: WireMergeReport,
    },
    Persisted {
        version: VersionVector,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct WireMergeReport {
    inserted: usize,
    already_present: usize,
}

impl From<MergeReport> for WireMergeReport {
    fn from(report: MergeReport) -> Self {
        Self {
            inserted: report.inserted,
            already_present: report.already_present,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SignedEnvelope {
    sequence: u64,
    payload: SyncPayload,
    mac: [u8; MAC_BYTES],
}

pub(crate) fn read_secret(path: &Path) -> std::io::Result<Vec<u8>> {
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
                "collaboration secret must not be a symlink: {}",
                path.display()
            )));
        }
        std::fs::File::open(path)?
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(invalid_input(format!(
            "collaboration secret must be a regular, non-symlink file: {}",
            path.display()
        )));
    }
    if metadata.len() as usize > MAX_SECRET_BYTES {
        return Err(invalid_input(format!(
            "collaboration secret exceeds {MAX_SECRET_BYTES} bytes"
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o077 != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "collaboration secret must not be readable or writable by group/others (use chmod 600)",
            ));
        }
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "collaboration secret must be owned by the current user",
            ));
        }
    }

    let mut secret = Vec::new();
    file.take((MAX_SECRET_BYTES + 1) as u64)
        .read_to_end(&mut secret)?;
    while matches!(secret.last(), Some(b'\n' | b'\r')) {
        secret.pop();
    }
    if secret.len() < MIN_SECRET_BYTES {
        return Err(invalid_input(format!(
            "collaboration secret must contain at least {MIN_SECRET_BYTES} bytes"
        )));
    }
    Ok(secret)
}

pub(crate) fn generate_secret(path: &Path) -> std::io::Result<()> {
    let mut random = [0_u8; NONCE_BYTES];
    getrandom::fill(&mut random).map_err(|error| std::io::Error::other(error.to_string()))?;
    let encoded = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut created = false;
    let result = (|| {
        let mut file = options.open(path)?;
        created = true;
        file.write_all(encoded.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        #[cfg(unix)]
        std::fs::File::open(
            path.parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new(".")),
        )?
        .sync_all()?;
        Ok(())
    })();
    if result.is_err() && created {
        let _ = std::fs::remove_file(path);
    }
    result
}

pub(crate) fn serve(
    bundle: &Path,
    bind: &str,
    secret_file: &Path,
    once: bool,
    ready_file: Option<&Path>,
) -> std::io::Result<()> {
    let address = loopback_address(bind)?;
    let listener = TcpListener::bind(address)?;
    serve_listener(bundle, listener, secret_file, once, ready_file)
}

fn serve_listener(
    bundle: &Path,
    listener: TcpListener,
    secret_file: &Path,
    once: bool,
    ready_file: Option<&Path>,
) -> std::io::Result<()> {
    let secret = read_secret(secret_file)?;
    let mut replica = collaboration_result(GraphReplica::load(bundle))?;
    let local_address = listener.local_addr()?;
    if !local_address.ip().is_loopback() {
        return Err(invalid_input("live collaboration must bind to loopback"));
    }
    if let Some(ready_file) = ready_file {
        write_ready_file(ready_file, local_address)?;
    }
    println!(
        "Live collaboration host {} listening on {local_address}",
        replica.actor()
    );
    println!("  transport: authenticated and integrity-protected; loopback only");

    loop {
        let (stream, peer_address) = listener.accept()?;
        match handle_peer(&mut replica, stream, &secret, bundle) {
            Ok(report) => {
                println!(
                    "  synchronized {} from {peer_address}: received {}, inserted {}, graph {} nodes / {} edges",
                    report.peer,
                    report.received_operations,
                    report.inserted_operations,
                    report.node_count,
                    report.edge_count
                );
            }
            Err(error) => eprintln!("  rejected peer {peer_address}: {error}"),
        }
        if once {
            return Ok(());
        }
    }
}

pub(crate) fn join(
    bundle: &Path,
    address: &str,
    secret_file: &Path,
    out: Option<&Path>,
) -> std::io::Result<LiveSyncReport> {
    let address = loopback_address(address)?;
    let secret = read_secret(secret_file)?;
    let mut replica = collaboration_result(GraphReplica::load(bundle))?;
    let mut stream = TcpStream::connect_timeout(&address, IO_TIMEOUT)?;
    configure_stream(&stream)?;

    let challenge: Challenge = read_frame(&mut stream, MAX_HANDSHAKE_BYTES)?;
    validate_challenge(&challenge)?;
    if challenge.server_actor == *replica.actor() {
        return Err(invalid_input(format!(
            "peer uses the same actor id '{}'; fork the bundle with a unique actor",
            replica.actor()
        )));
    }

    let client_nonce = random_nonce()?;
    let transcript = transcript_bytes(
        &challenge,
        replica.actor(),
        &client_nonce,
        replica.version(),
    )?;
    let proof = compute_mac(&secret, b"client-auth", &transcript)?;
    write_frame(
        &mut stream,
        &AuthRequest {
            client_actor: replica.actor().clone(),
            client_nonce,
            client_version: replica.version().clone(),
            proof,
        },
        MAX_HANDSHAKE_BYTES,
    )?;
    let response: AuthResponse = read_frame(&mut stream, MAX_HANDSHAKE_BYTES)?;
    if !response.accepted {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            response
                .error
                .unwrap_or_else(|| "peer rejected authentication".into()),
        ));
    }
    if response.server_actor != challenge.server_actor {
        return Err(invalid_data("server actor changed during authentication"));
    }
    verify_mac(&secret, b"server-auth", &transcript, &response.proof)?;
    let session_key = compute_mac(&secret, b"session-key", &transcript)?;

    let server_payload = read_signed(&mut stream, &session_key, b"server", 1)?;
    let (server_delta, server_version) = match server_payload {
        SyncPayload::ServerDelta { delta, version } => (delta, version),
        SyncPayload::Error { message } => return Err(invalid_data(message)),
        _ => return Err(invalid_data("expected server delta")),
    };
    let received_operations = server_delta.len();
    let inserted_operations = collaboration_result(replica.apply_delta(&server_delta))?.inserted;
    let client_delta = collaboration_result(replica.delta_since(&server_version))?;
    let sent_operations = client_delta.len();
    write_signed(
        &mut stream,
        &session_key,
        b"client",
        1,
        SyncPayload::ClientDelta {
            delta: client_delta,
        },
    )?;

    let ack = read_signed(&mut stream, &session_key, b"server", 2)?;
    let SyncPayload::Ack { version, report: _ } = ack else {
        if let SyncPayload::Error { message } = ack {
            return Err(invalid_data(message));
        }
        return Err(invalid_data("expected synchronization acknowledgement"));
    };
    if &version != replica.version() {
        return Err(invalid_data(
            "peer acknowledgement does not match the converged version",
        ));
    }
    let graph = collaboration_result(replica.materialize())?;
    collaboration_result(replica.acknowledge(challenge.server_actor.clone(), version.clone()))?;
    collaboration_result(replica.save(out.unwrap_or(bundle)))?;
    write_signed(
        &mut stream,
        &session_key,
        b"client",
        2,
        SyncPayload::Persisted { version },
    )?;
    Ok(LiveSyncReport {
        peer: challenge.server_actor,
        sent_operations,
        received_operations,
        inserted_operations,
        node_count: graph.node_count(),
        edge_count: graph.edge_count(),
    })
}

fn handle_peer(
    replica: &mut GraphReplica,
    mut stream: TcpStream,
    secret: &[u8],
    bundle: &Path,
) -> std::io::Result<LiveSyncReport> {
    configure_stream(&stream)?;
    let challenge = Challenge {
        magic: PROTOCOL_MAGIC.into(),
        version: PROTOCOL_VERSION,
        server_actor: replica.actor().clone(),
        server_nonce: random_nonce()?,
    };
    write_frame(&mut stream, &challenge, MAX_HANDSHAKE_BYTES)?;
    let request: AuthRequest = read_frame(&mut stream, MAX_HANDSHAKE_BYTES)?;

    let validated_actor = ActorId::new(request.client_actor.as_str())
        .map_err(|error| invalid_data(error.to_string()))?;
    if validated_actor == *replica.actor() {
        write_frame(
            &mut stream,
            &AuthResponse {
                accepted: false,
                server_actor: replica.actor().clone(),
                proof: [0; MAC_BYTES],
                error: Some("peer actor id must be unique".into()),
            },
            MAX_HANDSHAKE_BYTES,
        )?;
        return Err(invalid_input("peer actor id must be unique"));
    }
    let transcript = transcript_bytes(
        &challenge,
        &validated_actor,
        &request.client_nonce,
        &request.client_version,
    )?;
    if verify_mac(secret, b"client-auth", &transcript, &request.proof).is_err() {
        write_frame(
            &mut stream,
            &AuthResponse {
                accepted: false,
                server_actor: replica.actor().clone(),
                proof: [0; MAC_BYTES],
                error: Some("authentication failed".into()),
            },
            MAX_HANDSHAKE_BYTES,
        )?;
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "authentication failed",
        ));
    }
    let server_proof = compute_mac(secret, b"server-auth", &transcript)?;
    write_frame(
        &mut stream,
        &AuthResponse {
            accepted: true,
            server_actor: replica.actor().clone(),
            proof: server_proof,
            error: None,
        },
        MAX_HANDSHAKE_BYTES,
    )?;
    let session_key = compute_mac(secret, b"session-key", &transcript)?;

    let server_delta = match replica.delta_since(&request.client_version) {
        Ok(delta) => delta,
        Err(error) => {
            let _ = write_signed(
                &mut stream,
                &session_key,
                b"server",
                1,
                SyncPayload::Error {
                    message: error.to_string(),
                },
            );
            return Err(std::io::Error::other(error));
        }
    };
    let sent_operations = server_delta.len();
    write_signed(
        &mut stream,
        &session_key,
        b"server",
        1,
        SyncPayload::ServerDelta {
            delta: server_delta,
            version: replica.version().clone(),
        },
    )?;
    let client_payload = read_signed(&mut stream, &session_key, b"client", 1)?;
    let client_delta = match client_payload {
        SyncPayload::ClientDelta { delta } => delta,
        SyncPayload::Error { message } => return Err(invalid_data(message)),
        _ => return Err(invalid_data("expected client delta")),
    };
    let received_operations = client_delta.len();
    let mut staged = replica.clone();
    let merge = match staged.apply_delta(&client_delta) {
        Ok(report) => report,
        Err(error) => {
            let _ = write_signed(
                &mut stream,
                &session_key,
                b"server",
                2,
                SyncPayload::Error {
                    message: error.to_string(),
                },
            );
            return Err(invalid_data(error.to_string()));
        }
    };
    let graph = collaboration_result(staged.materialize())?;
    if let Err(error) = staged.save(bundle) {
        let _ = write_signed(
            &mut stream,
            &session_key,
            b"server",
            2,
            SyncPayload::Error {
                message: error.to_string(),
            },
        );
        return Err(std::io::Error::other(error));
    }
    *replica = staged;
    write_signed(
        &mut stream,
        &session_key,
        b"server",
        2,
        SyncPayload::Ack {
            version: replica.version().clone(),
            report: merge.into(),
        },
    )?;
    let persisted = read_signed(&mut stream, &session_key, b"client", 2)?;
    let SyncPayload::Persisted { version } = persisted else {
        return Err(invalid_data("expected durable peer acknowledgement"));
    };
    if &version != replica.version() {
        return Err(invalid_data(
            "peer durable acknowledgement does not match the converged version",
        ));
    }
    let mut acknowledged = replica.clone();
    collaboration_result(acknowledged.acknowledge(validated_actor.clone(), version))?;
    collaboration_result(acknowledged.save(bundle))?;
    *replica = acknowledged;
    Ok(LiveSyncReport {
        peer: validated_actor,
        sent_operations,
        received_operations,
        inserted_operations: merge.inserted,
        node_count: graph.node_count(),
        edge_count: graph.edge_count(),
    })
}

fn write_ready_file(path: &Path, address: SocketAddr) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let nonce = random_nonce()?
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let temporary = parent.join(format!(".bitcode-ready-{}-{nonce}.tmp", std::process::id()));
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
        writeln!(file, "{address}")?;
        file.sync_all()?;
        std::fs::hard_link(&temporary, path)?;
        published = true;
        #[cfg(unix)]
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

fn validate_challenge(challenge: &Challenge) -> std::io::Result<()> {
    if challenge.magic != PROTOCOL_MAGIC {
        return Err(invalid_data("bad live collaboration protocol magic"));
    }
    if challenge.version != PROTOCOL_VERSION {
        return Err(invalid_data(format!(
            "unsupported live collaboration protocol {}; expected {PROTOCOL_VERSION}",
            challenge.version
        )));
    }
    ActorId::new(challenge.server_actor.as_str())
        .map(|_| ())
        .map_err(|error| invalid_data(error.to_string()))
}

fn transcript_bytes(
    challenge: &Challenge,
    client_actor: &ActorId,
    client_nonce: &[u8; NONCE_BYTES],
    client_version: &VersionVector,
) -> std::io::Result<Vec<u8>> {
    serde_json::to_vec(&AuthTranscript {
        magic: PROTOCOL_MAGIC,
        version: PROTOCOL_VERSION,
        server_actor: &challenge.server_actor,
        client_actor,
        server_nonce: &challenge.server_nonce,
        client_nonce,
        client_version,
    })
    .map_err(|error| invalid_data(error.to_string()))
}

fn compute_mac(secret: &[u8], tag: &[u8], message: &[u8]) -> std::io::Result<[u8; MAC_BYTES]> {
    let mut mac = HmacSha256::new_from_slice(secret)
        .map_err(|_| invalid_input("invalid collaboration secret"))?;
    mac.update(tag);
    mac.update(message);
    Ok(mac.finalize().into_bytes().into())
}

fn verify_mac(
    secret: &[u8],
    tag: &[u8],
    message: &[u8],
    expected: &[u8; MAC_BYTES],
) -> std::io::Result<()> {
    let mut mac = HmacSha256::new_from_slice(secret)
        .map_err(|_| invalid_input("invalid collaboration secret"))?;
    mac.update(tag);
    mac.update(message);
    mac.verify_slice(expected)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::PermissionDenied, "invalid MAC"))
}

fn write_signed(
    stream: &mut TcpStream,
    key: &[u8],
    direction: &[u8],
    sequence: u64,
    payload: SyncPayload,
) -> std::io::Result<()> {
    let payload_bytes =
        serde_json::to_vec(&payload).map_err(|error| invalid_data(error.to_string()))?;
    let mut authenticated = sequence.to_be_bytes().to_vec();
    authenticated.extend_from_slice(&payload_bytes);
    let mac = compute_mac(key, direction, &authenticated)?;
    write_frame(
        stream,
        &SignedEnvelope {
            sequence,
            payload,
            mac,
        },
        MAX_SYNC_BYTES,
    )
}

fn read_signed(
    stream: &mut TcpStream,
    key: &[u8],
    direction: &[u8],
    expected_sequence: u64,
) -> std::io::Result<SyncPayload> {
    let envelope: SignedEnvelope = read_frame(stream, MAX_SYNC_BYTES)?;
    if envelope.sequence != expected_sequence {
        return Err(invalid_data(format!(
            "unexpected message sequence {}; expected {expected_sequence}",
            envelope.sequence
        )));
    }
    let payload_bytes =
        serde_json::to_vec(&envelope.payload).map_err(|error| invalid_data(error.to_string()))?;
    let mut authenticated = envelope.sequence.to_be_bytes().to_vec();
    authenticated.extend_from_slice(&payload_bytes);
    verify_mac(key, direction, &authenticated, &envelope.mac)?;
    Ok(envelope.payload)
}

fn write_frame<T: Serialize>(
    stream: &mut TcpStream,
    value: &T,
    limit: usize,
) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|error| invalid_data(error.to_string()))?;
    if bytes.is_empty() || bytes.len() > limit || bytes.len() > u32::MAX as usize {
        return Err(invalid_data(format!(
            "outgoing collaboration frame exceeds {limit} bytes"
        )));
    }
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)?;
    stream.flush()
}

fn read_frame<T: DeserializeOwned>(stream: &mut TcpStream, limit: usize) -> std::io::Result<T> {
    let mut length = [0_u8; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > limit {
        return Err(invalid_data(format!(
            "incoming collaboration frame is {length} bytes; limit is {limit}"
        )));
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(|error| invalid_data(error.to_string()))
}

fn random_nonce() -> std::io::Result<[u8; NONCE_BYTES]> {
    let mut nonce = [0; NONCE_BYTES];
    getrandom::fill(&mut nonce).map_err(|error| std::io::Error::other(error.to_string()))?;
    Ok(nonce)
}

fn configure_stream(stream: &TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    stream.set_nodelay(true)
}

fn loopback_address(value: &str) -> std::io::Result<SocketAddr> {
    let address: SocketAddr = value
        .parse()
        .map_err(|error| invalid_input(format!("invalid socket address '{value}': {error}")))?;
    if !address.ip().is_loopback() {
        return Err(invalid_input(
            "live collaboration accepts loopback addresses only; use an encrypted tunnel for remote peers",
        ));
    }
    Ok(address)
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
    use aether_graph::{Edge, EdgeKind, Node, NodeKind, SemanticGraph};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let name = format!(
                "bitcode-live-collab-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            );
            let path = std::env::temp_dir().join(name);
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn file(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn actor(name: &str) -> ActorId {
        ActorId::new(name).unwrap()
    }

    fn graph(source: &str) -> SemanticGraph {
        let mut graph = SemanticGraph::new();
        let module = graph.upsert_node(Node::new(NodeKind::Module, "app", "crate::app"));
        let run = graph.upsert_node(
            Node::new(NodeKind::Function, "run", "crate::app::run").with_source(source),
        );
        graph
            .add_edge(module, run, Edge::new(EdgeKind::Contains))
            .unwrap();
        graph
    }

    fn write_secret(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    #[test]
    fn authenticated_peers_exchange_concurrent_graph_deltas() {
        let temp = TempDir::new();
        let secret = temp.file("secret");
        write_secret(&secret, b"0123456789abcdef0123456789abcdef");
        let server_path = temp.file("alice.aetherc");
        let client_path = temp.file("bob.aetherc");
        let base = graph("fn run() {}");
        let mut alice = GraphReplica::from_graph(actor("alice"), &base);
        let mut bob = alice.fork(actor("bob")).unwrap();
        alice.sync_graph(&graph("fn run() { alice(); }")).unwrap();
        bob.sync_graph(&graph("fn run() { bob(); }")).unwrap();
        alice.save(&server_path).unwrap();
        bob.save(&client_path).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_bundle = server_path.clone();
        let server_secret = secret.clone();
        let host = thread::spawn(move || {
            serve_listener(&server_bundle, listener, &server_secret, true, None).unwrap()
        });
        let report = join(
            &client_path,
            &address.to_string(),
            &secret,
            Some(&client_path),
        )
        .unwrap();
        host.join().unwrap();

        assert_eq!(report.peer, actor("alice"));
        assert_eq!(report.sent_operations, 1);
        assert_eq!(report.received_operations, 1);
        let mut server = GraphReplica::load(&server_path).unwrap();
        let client = GraphReplica::load(&client_path).unwrap();
        assert_eq!(
            server
                .acknowledgements()
                .find(|(peer, _)| peer.as_str() == "bob")
                .map(|(_, version)| version),
            Some(server.version())
        );
        assert_eq!(
            client
                .acknowledgements()
                .find(|(peer, _)| peer.as_str() == "alice")
                .map(|(_, version)| version),
            Some(client.version())
        );
        assert_eq!(
            server.materialize().unwrap().to_ron().unwrap(),
            client.materialize().unwrap().to_ron().unwrap()
        );
        let expected = server.materialize().unwrap().to_ron().unwrap();
        assert!(server.compact_acknowledged().unwrap().removed_operations > 0);
        assert_eq!(server.materialize().unwrap().to_ron().unwrap(), expected);
    }

    #[test]
    fn wrong_secret_is_rejected_without_mutating_the_host() {
        let temp = TempDir::new();
        let server_secret = temp.file("server-secret");
        let client_secret = temp.file("client-secret");
        write_secret(&server_secret, b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        write_secret(&client_secret, b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let server_path = temp.file("alice.aetherc");
        let client_path = temp.file("bob.aetherc");
        let alice = GraphReplica::from_graph(actor("alice"), &graph("fn run() {}"));
        let bob = alice.fork(actor("bob")).unwrap();
        alice.save(&server_path).unwrap();
        bob.save(&client_path).unwrap();
        let before = std::fs::read(&server_path).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_bundle = server_path.clone();
        let host = thread::spawn(move || {
            serve_listener(&server_bundle, listener, &server_secret, true, None).unwrap()
        });
        assert!(join(&client_path, &address.to_string(), &client_secret, None).is_err());
        host.join().unwrap();
        assert_eq!(std::fs::read(&server_path).unwrap(), before);
    }

    #[test]
    fn peers_older_than_compacted_history_are_rejected_without_host_mutation() {
        let temp = TempDir::new();
        let secret = temp.file("secret");
        write_secret(&secret, b"0123456789abcdef0123456789abcdef");
        let server_path = temp.file("alice.aetherc");
        let stale_path = temp.file("charlie.aetherc");
        let base = graph("fn run() {}");
        let mut alice = GraphReplica::from_graph(actor("alice"), &base);
        let charlie = alice.fork(actor("charlie")).unwrap();
        let mut bob = alice.fork(actor("bob")).unwrap();
        alice.sync_graph(&graph("fn run() { first(); }")).unwrap();
        alice.sync_graph(&graph("fn run() { second(); }")).unwrap();
        bob.apply_delta(&alice.delta_since(bob.version()).unwrap())
            .unwrap();
        alice
            .acknowledge(actor("bob"), bob.version().clone())
            .unwrap();
        assert!(alice.compact_acknowledged().unwrap().removed_operations > 0);
        alice.save(&server_path).unwrap();
        charlie.save(&stale_path).unwrap();
        let before = std::fs::read(&server_path).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_bundle = server_path.clone();
        let server_secret = secret.clone();
        let host = thread::spawn(move || {
            serve_listener(&server_bundle, listener, &server_secret, true, None).unwrap()
        });
        let error = join(
            &stale_path,
            &address.to_string(),
            &secret,
            Some(&stale_path),
        )
        .unwrap_err();
        assert!(error.to_string().contains("predates compacted"));
        host.join().unwrap();
        assert_eq!(std::fs::read(&server_path).unwrap(), before);
    }

    #[test]
    fn signed_messages_reject_tampering_and_wrong_sequence() {
        fn assert_rejected(envelope: SignedEnvelope, key: &[u8], expected_sequence: u64) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let writer = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                write_frame(&mut stream, &envelope, MAX_SYNC_BYTES).unwrap();
            });
            let mut stream = TcpStream::connect(address).unwrap();
            assert!(read_signed(&mut stream, key, b"server", expected_sequence).is_err());
            writer.join().unwrap();
        }

        let key = b"0123456789abcdef0123456789abcdef";
        let payload = SyncPayload::Error {
            message: "original".into(),
        };
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        let mut authenticated = 1_u64.to_be_bytes().to_vec();
        authenticated.extend_from_slice(&payload_bytes);
        let mac = compute_mac(key, b"server", &authenticated).unwrap();
        let mut envelope = SignedEnvelope {
            sequence: 1,
            payload,
            mac,
        };
        envelope.payload = SyncPayload::Error {
            message: "tampered".into(),
        };
        assert_rejected(envelope, key, 1);

        let payload = SyncPayload::Error {
            message: "valid but out of order".into(),
        };
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        let mut authenticated = 2_u64.to_be_bytes().to_vec();
        authenticated.extend_from_slice(&payload_bytes);
        let envelope = SignedEnvelope {
            sequence: 2,
            payload,
            mac: compute_mac(key, b"server", &authenticated).unwrap(),
        };
        assert_rejected(envelope, key, 1);
    }

    #[test]
    fn frame_limit_is_checked_before_payload_allocation() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let writer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .write_all(&((MAX_HANDSHAKE_BYTES + 1) as u32).to_be_bytes())
                .unwrap();
        });
        let mut stream = TcpStream::connect(address).unwrap();
        assert!(read_frame::<Challenge>(&mut stream, MAX_HANDSHAKE_BYTES).is_err());
        writer.join().unwrap();
    }

    #[test]
    fn ready_files_publish_complete_addresses_without_overwriting() {
        let temp = TempDir::new();
        let ready = temp.file("host.ready");
        let address = "127.0.0.1:43210".parse().unwrap();
        write_ready_file(&ready, address).unwrap();
        let original = std::fs::read(&ready).unwrap();
        assert_eq!(original, b"127.0.0.1:43210\n");
        assert_eq!(
            write_ready_file(&ready, address).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(&ready).unwrap(), original);
    }

    #[test]
    fn secret_files_require_private_permissions_and_minimum_length() {
        let temp = TempDir::new();
        let secret = temp.file("secret");
        write_secret(&secret, b"short");
        assert!(read_secret(&secret).is_err());

        write_secret(&secret, b"0123456789abcdef0123456789abcdef");
        assert_eq!(read_secret(&secret).unwrap().len(), 32);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(
                read_secret(&secret).unwrap_err().kind(),
                std::io::ErrorKind::PermissionDenied
            );
        }
    }

    #[test]
    fn generated_secrets_are_private_strong_and_never_overwritten() {
        let temp = TempDir::new();
        let secret = temp.file("generated-secret");
        generate_secret(&secret).unwrap();
        assert_eq!(read_secret(&secret).unwrap().len(), 64);
        let original = std::fs::read(&secret).unwrap();
        assert_eq!(
            generate_secret(&secret).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(&secret).unwrap(), original);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&secret).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn remote_addresses_are_rejected_without_network_access() {
        assert!(loopback_address("0.0.0.0:9000").is_err());
        assert!(loopback_address("192.0.2.1:9000").is_err());
        assert!(loopback_address("127.0.0.1:9000").is_ok());
        assert!(loopback_address("[::1]:9000").is_ok());
    }
}
