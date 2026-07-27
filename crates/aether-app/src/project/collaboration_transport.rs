//! Authenticated, bounded live transport for graph collaboration replicas.
//!
//! The protocol deliberately binds only loopback sockets. Remote peers should
//! use an authenticated encrypted tunnel (for example SSH) because the session
//! provides mutual authentication and integrity, but not confidentiality.
//! Authentication proves possession of the group secret; optional pinned
//! Ed25519 identities authenticate ephemeral X25519 key agreement so a different
//! group member cannot derive that session's integrity key. Both sides also
//! require the authenticated actor to be active in the causal membership roster.

use super::collaboration_discovery::DiscoveryLease;
use super::collaboration_identity::{SessionIdentity, PUBLIC_KEY_BYTES};
use aether_graph::{ActorId, GraphDelta, GraphError, GraphReplica, MergeReport, VersionVector};
use hmac::{Hmac, Mac};
use ring::agreement;
use ring::rand::SystemRandom;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::Sha256;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;

const PROTOCOL_MAGIC: &str = "BITCODE_LIVE_COLLAB";
pub(super) const PROTOCOL_VERSION: u32 = 5;
const NONCE_BYTES: usize = 32;
const MAC_BYTES: usize = 32;
const KEY_AGREEMENT_BYTES: usize = 32;
const MIN_SECRET_BYTES: usize = 32;
const MAX_SECRET_BYTES: usize = 4096;
const MAX_PRESENCE_BYTES: usize = 256;
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
    peer_presence: SessionPresence,
    peer_identity_fingerprint: Option<String>,
}

impl LiveSyncReport {
    pub(crate) fn peer_presence(&self) -> Option<&str> {
        self.peer_presence.status()
    }

    pub(crate) fn peer_identity_fingerprint(&self) -> Option<&str> {
        self.peer_identity_fingerprint.as_deref()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ServeOptions<'a> {
    pub(crate) once: bool,
    pub(crate) ready_file: Option<&'a Path>,
    pub(crate) presence: Option<&'a str>,
    pub(crate) discovery_directory: Option<&'a Path>,
    pub(crate) identity_file: Option<&'a Path>,
    pub(crate) trust_store: Option<&'a Path>,
}

struct ServeListenerOptions<'a> {
    once: bool,
    ready_file: Option<&'a Path>,
    presence: SessionPresence,
    discovery_directory: Option<&'a Path>,
    identity_file: Option<&'a Path>,
    trust_store: Option<&'a Path>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
struct SessionPresence(Option<String>);

impl SessionPresence {
    fn new(status: Option<&str>) -> std::io::Result<Self> {
        let status = status.map(str::trim).filter(|status| !status.is_empty());
        let presence: SessionPresence = Self(status.map(str::to_owned));
        presence
            .validate()
            .map_err(|error| invalid_input(error.to_string()))?;
        Ok(presence)
    }

    fn validate(&self) -> std::io::Result<()> {
        let Some(status) = self.status() else {
            return Ok(());
        };
        if status.is_empty() || status.trim() != status {
            return Err(invalid_data(
                "session presence must use a non-empty canonical status",
            ));
        }
        if status.len() > MAX_PRESENCE_BYTES {
            return Err(invalid_data(format!(
                "session presence exceeds {MAX_PRESENCE_BYTES} UTF-8 bytes"
            )));
        }
        for character in status.chars() {
            if is_unsafe_presence_character(character) {
                return Err(invalid_data(
                    "session presence must be single-line text without control or directional formatting characters",
                ));
            }
        }
        Ok(())
    }

    fn status(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

impl std::fmt::Display for SessionPresence {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.status() {
            Some(status) => formatter.write_str(status),
            None => formatter.write_str("online (no status shared)"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Challenge {
    magic: String,
    version: u32,
    server_actor: ActorId,
    server_nonce: [u8; NONCE_BYTES],
    server_presence: SessionPresence,
    server_identity: Option<[u8; PUBLIC_KEY_BYTES]>,
    server_key_agreement: Option<[u8; KEY_AGREEMENT_BYTES]>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthRequest {
    client_actor: ActorId,
    client_nonce: [u8; NONCE_BYTES],
    client_version: VersionVector,
    client_presence: SessionPresence,
    client_identity: Option<[u8; PUBLIC_KEY_BYTES]>,
    client_key_agreement: Option<[u8; KEY_AGREEMENT_BYTES]>,
    identity_proof: Option<Vec<u8>>,
    proof: [u8; MAC_BYTES],
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthResponse {
    accepted: bool,
    server_actor: ActorId,
    proof: [u8; MAC_BYTES],
    identity_proof: Option<Vec<u8>>,
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
    server_presence: &'a SessionPresence,
    client_presence: &'a SessionPresence,
    server_identity: &'a Option<[u8; PUBLIC_KEY_BYTES]>,
    client_identity: &'a Option<[u8; PUBLIC_KEY_BYTES]>,
    server_key_agreement: &'a Option<[u8; KEY_AGREEMENT_BYTES]>,
    client_key_agreement: &'a Option<[u8; KEY_AGREEMENT_BYTES]>,
}

struct EphemeralKeyAgreement {
    private_key: agreement::EphemeralPrivateKey,
    public_key: [u8; KEY_AGREEMENT_BYTES],
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

fn load_session_identity(
    identity_file: Option<&Path>,
    trust_store: Option<&Path>,
    actor: &ActorId,
) -> std::io::Result<Option<SessionIdentity>> {
    match (identity_file, trust_store) {
        (Some(identity_file), Some(trust_store)) => {
            let identity = SessionIdentity::load(identity_file, trust_store, actor)?;
            if identity.actor() != actor {
                return Err(invalid_data("loaded actor identity changed unexpectedly"));
            }
            Ok(Some(identity))
        }
        (None, None) => Ok(None),
        _ => Err(invalid_input(
            "--identity-file and --trust-store must be provided together",
        )),
    }
}

fn generate_key_agreement() -> std::io::Result<EphemeralKeyAgreement> {
    let private_key =
        agreement::EphemeralPrivateKey::generate(&agreement::X25519, &SystemRandom::new())
            .map_err(|_| std::io::Error::other("failed to generate ephemeral X25519 key"))?;
    let public_key: [u8; KEY_AGREEMENT_BYTES] = private_key
        .compute_public_key()
        .map_err(|_| std::io::Error::other("failed to derive ephemeral X25519 public key"))?
        .as_ref()
        .try_into()
        .map_err(|_| invalid_data("unexpected X25519 public-key length"))?;
    Ok(EphemeralKeyAgreement {
        private_key,
        public_key,
    })
}

fn identity_session_key(
    secret: &[u8],
    transcript: &[u8],
    private_key: agreement::EphemeralPrivateKey,
    peer_public_key: &[u8; KEY_AGREEMENT_BYTES],
) -> std::io::Result<[u8; MAC_BYTES]> {
    let peer_public_key =
        agreement::UnparsedPublicKey::new(&agreement::X25519, peer_public_key.as_slice());
    agreement::agree_ephemeral(private_key, &peer_public_key, |shared_secret| {
        let mut key_material =
            Vec::with_capacity(transcript.len() + shared_secret.len() + 2 * size_of::<u64>());
        key_material.extend_from_slice(&(transcript.len() as u64).to_be_bytes());
        key_material.extend_from_slice(transcript);
        key_material.extend_from_slice(&(shared_secret.len() as u64).to_be_bytes());
        key_material.extend_from_slice(shared_secret);
        compute_mac(secret, b"identity-session-key-v1", &key_material)
    })
    .map_err(|_| permission_denied("invalid peer X25519 key agreement"))?
}

pub(crate) fn serve(
    bundle: &Path,
    bind: &str,
    secret_file: &Path,
    options: ServeOptions<'_>,
) -> std::io::Result<()> {
    let presence = SessionPresence::new(options.presence)?;
    let address = loopback_address(bind)?;
    let listener = TcpListener::bind(address)?;
    serve_listener(
        bundle,
        listener,
        secret_file,
        ServeListenerOptions {
            once: options.once,
            ready_file: options.ready_file,
            presence,
            discovery_directory: options.discovery_directory,
            identity_file: options.identity_file,
            trust_store: options.trust_store,
        },
    )
}

fn serve_listener(
    bundle: &Path,
    listener: TcpListener,
    secret_file: &Path,
    options: ServeListenerOptions<'_>,
) -> std::io::Result<()> {
    let secret = read_secret(secret_file)?;
    let mut replica = collaboration_result(GraphReplica::load(bundle))?;
    if !collaboration_result(replica.is_member(replica.actor()))? {
        return Err(invalid_input(format!(
            "host actor '{}' is not an active collaboration member",
            replica.actor()
        )));
    }
    let identity =
        load_session_identity(options.identity_file, options.trust_store, replica.actor())?;
    let local_address = listener.local_addr()?;
    if !local_address.ip().is_loopback() {
        return Err(invalid_input("live collaboration must bind to loopback"));
    }
    let discovery_lease = options
        .discovery_directory
        .map(|directory| {
            DiscoveryLease::publish(directory, replica.actor(), local_address, &secret)
        })
        .transpose()?;
    if let Some(ready_file) = options.ready_file {
        write_ready_file(ready_file, local_address)?;
    }
    println!(
        "Live collaboration host {} listening on {local_address}",
        replica.actor()
    );
    println!("  transport: authenticated and integrity-protected; loopback only");
    println!("  session presence: {}", options.presence);
    match identity.as_ref() {
        Some(identity) => println!(
            "  actor identity: Ed25519 SHA-256 {}",
            identity.fingerprint()
        ),
        None => println!("  actor identity: group-secret only (legacy mode)"),
    }
    if let Some(lease) = discovery_lease.as_ref() {
        println!("  local discovery: {}", lease.path().display());
    }

    loop {
        let (stream, peer_address) = listener.accept()?;
        match handle_peer(
            &mut replica,
            stream,
            &secret,
            bundle,
            &options.presence,
            identity.as_ref(),
        ) {
            Ok(report) => {
                println!(
                    "  synchronized {} from {peer_address}: received {}, inserted {}, graph {} nodes / {} edges",
                    report.peer,
                    report.received_operations,
                    report.inserted_operations,
                    report.node_count,
                    report.edge_count
                );
                println!("    peer presence: {}", report.peer_presence);
                match report.peer_identity_fingerprint() {
                    Some(fingerprint) => {
                        println!("    peer identity: Ed25519 SHA-256 {fingerprint}")
                    }
                    None => println!("    peer identity: group-secret only (legacy mode)"),
                }
            }
            Err(error) => eprintln!("  rejected peer {peer_address}: {error}"),
        }
        if options.once {
            return Ok(());
        }
    }
}

pub(crate) fn join(
    bundle: &Path,
    address: &str,
    secret_file: &Path,
    out: Option<&Path>,
    presence: Option<&str>,
    identity_file: Option<&Path>,
    trust_store: Option<&Path>,
) -> std::io::Result<LiveSyncReport> {
    let presence = SessionPresence::new(presence)?;
    let address = loopback_address(address)?;
    let secret = read_secret(secret_file)?;
    let mut replica = collaboration_result(GraphReplica::load(bundle))?;
    if !collaboration_result(replica.is_member(replica.actor()))? {
        return Err(invalid_input(format!(
            "local actor '{}' is not an active collaboration member",
            replica.actor()
        )));
    }
    let identity = load_session_identity(identity_file, trust_store, replica.actor())?;
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
    if !collaboration_result(replica.is_member(&challenge.server_actor))? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "peer '{}' is not an active collaboration member",
                challenge.server_actor
            ),
        ));
    }
    match (
        identity.as_ref(),
        challenge.server_identity.as_ref(),
        challenge.server_key_agreement.as_ref(),
    ) {
        (Some(_), Some(_), Some(_)) | (None, None, None) => {}
        (Some(_), None, None) => {
            return Err(permission_denied(
                "peer does not offer a pinned actor identity; refusing downgrade to group-secret-only authentication",
            ))
        }
        (Some(_), _, _) => {
            return Err(invalid_data(
                "peer offered an incomplete actor identity handshake",
            ))
        }
        (None, _, _) => {
            return Err(permission_denied(
                "peer requires actor identity authentication; provide --identity-file and --trust-store",
            ))
        }
    }

    let client_nonce = random_nonce()?;
    let client_identity = identity.as_ref().map(|identity| *identity.public_key());
    let key_agreement = identity
        .as_ref()
        .map(|_| generate_key_agreement())
        .transpose()?;
    let client_key_agreement = key_agreement
        .as_ref()
        .map(|key_agreement| key_agreement.public_key);
    let transcript = transcript_bytes(
        &challenge,
        replica.actor(),
        &client_nonce,
        replica.version(),
        &presence,
        &client_identity,
        &client_key_agreement,
    )?;
    let proof = compute_mac(&secret, b"client-auth", &transcript)?;
    let identity_proof = identity
        .as_ref()
        .map(|identity| identity.sign(b"client", &transcript));
    write_frame(
        &mut stream,
        &AuthRequest {
            client_actor: replica.actor().clone(),
            client_nonce,
            client_version: replica.version().clone(),
            client_presence: presence,
            client_identity,
            client_key_agreement,
            identity_proof,
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
    let peer_identity_fingerprint = match identity.as_ref() {
        Some(identity) => {
            let server_identity = challenge
                .server_identity
                .as_ref()
                .ok_or_else(|| invalid_data("server identity disappeared from challenge"))?;
            let signature = response
                .identity_proof
                .as_deref()
                .ok_or_else(|| permission_denied("server omitted its actor identity proof"))?;
            Some(identity.verify_peer(
                &challenge.server_actor,
                server_identity,
                b"server",
                &transcript,
                signature,
            )?)
        }
        None => {
            if response.identity_proof.is_some() {
                return Err(invalid_data(
                    "legacy authentication response unexpectedly included an identity proof",
                ));
            }
            None
        }
    };
    let session_key = match (key_agreement, challenge.server_key_agreement.as_ref()) {
        (Some(key_agreement), Some(peer_public_key)) => identity_session_key(
            &secret,
            &transcript,
            key_agreement.private_key,
            peer_public_key,
        )?,
        (None, None) => compute_mac(&secret, b"session-key", &transcript)?,
        _ => return Err(invalid_data("actor identity key agreement changed mode")),
    };

    let server_payload = read_signed(&mut stream, &session_key, b"server", 1)?;
    let (server_delta, server_version) = match server_payload {
        SyncPayload::ServerDelta { delta, version } => (delta, version),
        SyncPayload::Error { message } => return Err(invalid_data(message)),
        _ => return Err(invalid_data("expected server delta")),
    };
    let received_operations = server_delta.len();
    let inserted_operations = collaboration_result(replica.apply_delta(&server_delta))?.inserted;
    ensure_session_members_active(&replica, &challenge.server_actor)?;
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
        peer_presence: challenge.server_presence,
        peer_identity_fingerprint,
    })
}

fn handle_peer(
    replica: &mut GraphReplica,
    mut stream: TcpStream,
    secret: &[u8],
    bundle: &Path,
    presence: &SessionPresence,
    identity: Option<&SessionIdentity>,
) -> std::io::Result<LiveSyncReport> {
    configure_stream(&stream)?;
    let key_agreement = identity.map(|_| generate_key_agreement()).transpose()?;
    let challenge = Challenge {
        magic: PROTOCOL_MAGIC.into(),
        version: PROTOCOL_VERSION,
        server_actor: replica.actor().clone(),
        server_nonce: random_nonce()?,
        server_presence: presence.clone(),
        server_identity: identity.map(|identity| *identity.public_key()),
        server_key_agreement: key_agreement
            .as_ref()
            .map(|key_agreement| key_agreement.public_key),
    };
    write_frame(&mut stream, &challenge, MAX_HANDSHAKE_BYTES)?;
    let request: AuthRequest = read_frame(&mut stream, MAX_HANDSHAKE_BYTES)?;

    let validated_actor = ActorId::new(request.client_actor.as_str())
        .map_err(|error| invalid_data(error.to_string()))?;
    request.client_presence.validate()?;
    if validated_actor == *replica.actor() {
        write_frame(
            &mut stream,
            &AuthResponse {
                accepted: false,
                server_actor: replica.actor().clone(),
                proof: [0; MAC_BYTES],
                identity_proof: None,
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
        &request.client_presence,
        &request.client_identity,
        &request.client_key_agreement,
    )?;
    if verify_mac(secret, b"client-auth", &transcript, &request.proof).is_err() {
        write_frame(
            &mut stream,
            &AuthResponse {
                accepted: false,
                server_actor: replica.actor().clone(),
                proof: [0; MAC_BYTES],
                identity_proof: None,
                error: Some("authentication failed".into()),
            },
            MAX_HANDSHAKE_BYTES,
        )?;
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "authentication failed",
        ));
    }
    if !collaboration_result(replica.is_member(&validated_actor))? {
        write_frame(
            &mut stream,
            &AuthResponse {
                accepted: false,
                server_actor: replica.actor().clone(),
                proof: [0; MAC_BYTES],
                identity_proof: None,
                error: Some(format!(
                    "actor '{validated_actor}' is not an active collaboration member"
                )),
            },
            MAX_HANDSHAKE_BYTES,
        )?;
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("actor '{validated_actor}' is not an active collaboration member"),
        ));
    }
    let peer_identity_fingerprint = match (
        identity,
        request.client_identity.as_ref(),
        request.client_key_agreement.as_ref(),
        request.identity_proof.as_deref(),
    ) {
        (Some(identity), Some(public_key), Some(_), Some(signature)) => {
            match identity.verify_peer(
                &validated_actor,
                public_key,
                b"client",
                &transcript,
                signature,
            ) {
                Ok(fingerprint) => Some(fingerprint),
                Err(error) => {
                    let message = error.to_string();
                    send_auth_rejection(&mut stream, replica.actor(), &message)?;
                    return Err(error);
                }
            }
        }
        (Some(_), _, _, _) => {
            let message =
                "host requires a complete actor identity proof and ephemeral key agreement";
            send_auth_rejection(&mut stream, replica.actor(), message)?;
            return Err(permission_denied(message));
        }
        (None, None, None, None) => None,
        (None, _, _, _) => {
            let message =
                "host is in group-secret-only mode and cannot authenticate a client identity";
            send_auth_rejection(&mut stream, replica.actor(), message)?;
            return Err(permission_denied(message));
        }
    };
    let session_key = match (key_agreement, request.client_key_agreement.as_ref()) {
        (Some(key_agreement), Some(peer_public_key)) => match identity_session_key(
            secret,
            &transcript,
            key_agreement.private_key,
            peer_public_key,
        ) {
            Ok(session_key) => session_key,
            Err(error) => {
                let message = error.to_string();
                send_auth_rejection(&mut stream, replica.actor(), &message)?;
                return Err(error);
            }
        },
        (None, None) => compute_mac(secret, b"session-key", &transcript)?,
        _ => {
            let message = "actor identity key agreement changed mode";
            send_auth_rejection(&mut stream, replica.actor(), message)?;
            return Err(invalid_data(message));
        }
    };
    let server_proof = compute_mac(secret, b"server-auth", &transcript)?;
    let identity_proof = identity.map(|identity| identity.sign(b"server", &transcript));
    write_frame(
        &mut stream,
        &AuthResponse {
            accepted: true,
            server_actor: replica.actor().clone(),
            proof: server_proof,
            identity_proof,
            error: None,
        },
        MAX_HANDSHAKE_BYTES,
    )?;

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
    if let Err(error) = ensure_session_members_active(&staged, &validated_actor) {
        let _ = write_signed(
            &mut stream,
            &session_key,
            b"server",
            2,
            SyncPayload::Error {
                message: error.to_string(),
            },
        );
        return Err(error);
    }
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
        peer_presence: request.client_presence,
        peer_identity_fingerprint,
    })
}

fn send_auth_rejection(
    stream: &mut TcpStream,
    server_actor: &ActorId,
    message: &str,
) -> std::io::Result<()> {
    write_frame(
        stream,
        &AuthResponse {
            accepted: false,
            server_actor: server_actor.clone(),
            proof: [0; MAC_BYTES],
            identity_proof: None,
            error: Some(message.into()),
        },
        MAX_HANDSHAKE_BYTES,
    )
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
        .map_err(|error| invalid_data(error.to_string()))?;
    if challenge.server_identity.is_some() != challenge.server_key_agreement.is_some() {
        return Err(invalid_data(
            "server identity and X25519 key agreement must be offered together",
        ));
    }
    challenge.server_presence.validate()
}

fn transcript_bytes(
    challenge: &Challenge,
    client_actor: &ActorId,
    client_nonce: &[u8; NONCE_BYTES],
    client_version: &VersionVector,
    client_presence: &SessionPresence,
    client_identity: &Option<[u8; PUBLIC_KEY_BYTES]>,
    client_key_agreement: &Option<[u8; KEY_AGREEMENT_BYTES]>,
) -> std::io::Result<Vec<u8>> {
    serde_json::to_vec(&AuthTranscript {
        magic: PROTOCOL_MAGIC,
        version: PROTOCOL_VERSION,
        server_actor: &challenge.server_actor,
        client_actor,
        server_nonce: &challenge.server_nonce,
        client_nonce,
        client_version,
        server_presence: &challenge.server_presence,
        client_presence,
        server_identity: &challenge.server_identity,
        client_identity,
        server_key_agreement: &challenge.server_key_agreement,
        client_key_agreement,
    })
    .map_err(|error| invalid_data(error.to_string()))
}

fn is_unsafe_presence_character(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
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

fn ensure_session_members_active(replica: &GraphReplica, peer: &ActorId) -> std::io::Result<()> {
    let local = replica.actor();
    if !collaboration_result(replica.is_member(local))?
        || !collaboration_result(replica.is_member(peer))?
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "live synchronization cannot remove authenticated session actors '{local}' or '{peer}'"
            ),
        ));
    }
    Ok(())
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
    use crate::project::collaboration_identity::{
        generate_identity, trust_identity, IdentitySummary,
    };
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

    fn generate_identity_files(
        temp: &TempDir,
        bundle: &Path,
        name: &str,
    ) -> (PathBuf, PathBuf, IdentitySummary) {
        let private = temp.file(&format!("{name}.identity"));
        let public = temp.file(&format!("{name}.identity.pub"));
        let summary = generate_identity(bundle, &private, &public).unwrap();
        (private, public, summary)
    }

    fn pin_identity(trust_store: &Path, public: &Path, identity: &IdentitySummary) {
        trust_identity(trust_store, public, &identity.fingerprint).unwrap();
    }

    fn once_listener_options() -> ServeListenerOptions<'static> {
        ServeListenerOptions {
            once: true,
            ready_file: None,
            presence: SessionPresence::default(),
            discovery_directory: None,
            identity_file: None,
            trust_store: None,
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
            serve_listener(
                &server_bundle,
                listener,
                &server_secret,
                once_listener_options(),
            )
            .unwrap()
        });
        let report = join(
            &client_path,
            &address.to_string(),
            &secret,
            Some(&client_path),
            None,
            None,
            None,
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
    fn authenticated_session_presence_is_bidirectional_and_ephemeral() {
        const ALICE_STATUS: &str = "alice-presence-sentinel: reviewing parser changes";
        const BOB_STATUS: &str = "bob-presence-sentinel: running transport tests";
        let temp = TempDir::new();
        let secret_path = temp.file("secret");
        let secret = b"0123456789abcdef0123456789abcdef";
        write_secret(&secret_path, secret);
        let server_path = temp.file("alice.aetherc");
        let client_path = temp.file("bob.aetherc");
        let base = graph("fn run() {}");
        let mut alice = GraphReplica::from_graph(actor("alice"), &base);
        let bob = alice.fork(actor("bob")).unwrap();
        alice.save(&server_path).unwrap();
        bob.save(&client_path).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_bundle = server_path.clone();
        let server_presence = SessionPresence::new(Some(ALICE_STATUS)).unwrap();
        let host = thread::spawn(move || {
            let mut replica = GraphReplica::load(&server_bundle).unwrap();
            let (stream, _) = listener.accept().unwrap();
            handle_peer(
                &mut replica,
                stream,
                secret,
                &server_bundle,
                &server_presence,
                None,
            )
        });
        let client_report = join(
            &client_path,
            &address.to_string(),
            &secret_path,
            Some(&client_path),
            Some(BOB_STATUS),
            None,
            None,
        )
        .unwrap();
        let server_report = host.join().unwrap().unwrap();

        assert_eq!(client_report.peer_presence(), Some(ALICE_STATUS));
        assert_eq!(server_report.peer_presence(), Some(BOB_STATUS));
        for bundle in [&server_path, &client_path] {
            let bytes = std::fs::read(bundle).unwrap();
            for status in [ALICE_STATUS, BOB_STATUS] {
                assert!(
                    !bytes
                        .windows(status.len())
                        .any(|window| window == status.as_bytes()),
                    "{} persisted ephemeral presence {status:?}",
                    bundle.display()
                );
            }
        }
    }

    #[test]
    fn pinned_identity_session_key_requires_ephemeral_private_keys() {
        let secret = b"0123456789abcdef0123456789abcdef";
        let transcript = b"signed identity handshake transcript";
        let server = generate_key_agreement().unwrap();
        let client = generate_key_agreement().unwrap();
        let server_public = server.public_key;
        let client_public = client.public_key;

        let server_key =
            identity_session_key(secret, transcript, server.private_key, &client_public).unwrap();
        let client_key =
            identity_session_key(secret, transcript, client.private_key, &server_public).unwrap();
        assert_eq!(server_key, client_key);
        assert_ne!(
            server_key,
            compute_mac(secret, b"session-key", transcript).unwrap()
        );

        let invalid = generate_key_agreement().unwrap();
        assert!(identity_session_key(
            secret,
            transcript,
            invalid.private_key,
            &[0; KEY_AGREEMENT_BYTES]
        )
        .is_err());
    }

    #[test]
    fn pinned_actor_identities_mutually_authenticate_the_live_session() {
        let temp = TempDir::new();
        let secret = temp.file("secret");
        write_secret(&secret, b"0123456789abcdef0123456789abcdef");
        let server_path = temp.file("alice.aetherc");
        let client_path = temp.file("bob.aetherc");
        let mut alice = GraphReplica::from_graph(actor("alice"), &graph("fn run() {}"));
        let mut bob = alice.fork(actor("bob")).unwrap();
        bob.sync_graph(&graph("fn run() { signed(); }")).unwrap();
        alice.save(&server_path).unwrap();
        bob.save(&client_path).unwrap();
        let (alice_private, alice_public, alice_identity) =
            generate_identity_files(&temp, &server_path, "alice");
        let (bob_private, bob_public, bob_identity) =
            generate_identity_files(&temp, &client_path, "bob");
        let alice_trust = temp.file("alice.trust");
        let bob_trust = temp.file("bob.trust");
        pin_identity(&alice_trust, &bob_public, &bob_identity);
        pin_identity(&bob_trust, &alice_public, &alice_identity);

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_bundle = server_path.clone();
        let server_secret = secret.clone();
        let host = thread::spawn(move || {
            serve_listener(
                &server_bundle,
                listener,
                &server_secret,
                ServeListenerOptions {
                    identity_file: Some(&alice_private),
                    trust_store: Some(&alice_trust),
                    ..once_listener_options()
                },
            )
            .unwrap()
        });
        let report = join(
            &client_path,
            &address.to_string(),
            &secret,
            Some(&client_path),
            None,
            Some(&bob_private),
            Some(&bob_trust),
        )
        .unwrap();
        host.join().unwrap();

        assert_eq!(
            report.peer_identity_fingerprint(),
            Some(alice_identity.fingerprint.as_str())
        );
        let server = GraphReplica::load(&server_path).unwrap();
        let client = GraphReplica::load(&client_path).unwrap();
        assert_eq!(
            server.materialize().unwrap().to_ron().unwrap(),
            client.materialize().unwrap().to_ron().unwrap()
        );
    }

    #[test]
    fn group_secret_holder_cannot_impersonate_a_pinned_client_actor() {
        let temp = TempDir::new();
        let secret_path = temp.file("secret");
        let secret = b"0123456789abcdef0123456789abcdef";
        write_secret(&secret_path, secret);
        let server_path = temp.file("alice.aetherc");
        let client_path = temp.file("bob.aetherc");
        let mut alice = GraphReplica::from_graph(actor("alice"), &graph("fn run() {}"));
        let bob = alice.fork(actor("bob")).unwrap();
        alice.save(&server_path).unwrap();
        bob.save(&client_path).unwrap();
        let before_server = std::fs::read(&server_path).unwrap();
        let before_client = std::fs::read(&client_path).unwrap();
        let (alice_private, alice_public, alice_identity) =
            generate_identity_files(&temp, &server_path, "alice");
        let (_bob_private, bob_public, bob_identity) =
            generate_identity_files(&temp, &client_path, "bob-genuine");
        let (attacker_private, _attacker_public, _attacker_identity) =
            generate_identity_files(&temp, &client_path, "bob-attacker");
        let alice_trust = temp.file("alice.trust");
        let bob_trust = temp.file("bob.trust");
        pin_identity(&alice_trust, &bob_public, &bob_identity);
        pin_identity(&bob_trust, &alice_public, &alice_identity);
        let alice_session =
            SessionIdentity::load(&alice_private, &alice_trust, &actor("alice")).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_bundle = server_path.clone();
        let host = thread::spawn(move || {
            let mut replica = GraphReplica::load(&server_bundle).unwrap();
            let (stream, _) = listener.accept().unwrap();
            handle_peer(
                &mut replica,
                stream,
                secret,
                &server_bundle,
                &SessionPresence::default(),
                Some(&alice_session),
            )
        });
        let error = join(
            &client_path,
            &address.to_string(),
            &secret_path,
            None,
            None,
            Some(&attacker_private),
            Some(&bob_trust),
        )
        .unwrap_err();
        assert!(error.to_string().contains("does not match"), "{error}");
        assert_eq!(
            host.join().unwrap().unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        assert_eq!(std::fs::read(&server_path).unwrap(), before_server);
        assert_eq!(std::fs::read(&client_path).unwrap(), before_client);
    }

    #[test]
    fn identity_mode_refuses_downgrade_in_either_direction() {
        let temp = TempDir::new();
        let secret = temp.file("secret");
        write_secret(&secret, b"0123456789abcdef0123456789abcdef");
        let server_path = temp.file("alice.aetherc");
        let client_path = temp.file("bob.aetherc");
        let mut alice = GraphReplica::from_graph(actor("alice"), &graph("fn run() {}"));
        let bob = alice.fork(actor("bob")).unwrap();
        alice.save(&server_path).unwrap();
        bob.save(&client_path).unwrap();
        let (alice_private, alice_public, alice_identity) =
            generate_identity_files(&temp, &server_path, "alice");
        let (bob_private, bob_public, bob_identity) =
            generate_identity_files(&temp, &client_path, "bob");
        let alice_trust = temp.file("alice.trust");
        let bob_trust = temp.file("bob.trust");
        pin_identity(&alice_trust, &bob_public, &bob_identity);
        pin_identity(&bob_trust, &alice_public, &alice_identity);

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_bundle = server_path.clone();
        let server_secret = secret.clone();
        let alice_private_for_host = alice_private.clone();
        let alice_trust_for_host = alice_trust.clone();
        let identity_host = thread::spawn(move || {
            serve_listener(
                &server_bundle,
                listener,
                &server_secret,
                ServeListenerOptions {
                    identity_file: Some(&alice_private_for_host),
                    trust_store: Some(&alice_trust_for_host),
                    ..once_listener_options()
                },
            )
            .unwrap()
        });
        let error = join(
            &client_path,
            &address.to_string(),
            &secret,
            None,
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("requires actor identity"),
            "{error}"
        );
        identity_host.join().unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_bundle = server_path.clone();
        let server_secret = secret.clone();
        let legacy_host = thread::spawn(move || {
            serve_listener(
                &server_bundle,
                listener,
                &server_secret,
                once_listener_options(),
            )
            .unwrap()
        });
        let error = join(
            &client_path,
            &address.to_string(),
            &secret,
            None,
            None,
            Some(&bob_private),
            Some(&bob_trust),
        )
        .unwrap_err();
        assert!(error.to_string().contains("refusing downgrade"), "{error}");
        legacy_host.join().unwrap();
    }

    #[test]
    fn session_presence_is_canonical_bounded_and_display_safe() {
        let canonical = SessionPresence::new(Some("  reviewing 🦀 changes  ")).unwrap();
        assert_eq!(canonical.status(), Some("reviewing 🦀 changes"));
        assert_eq!(canonical.to_string(), "reviewing 🦀 changes");
        assert_eq!(
            SessionPresence::default().to_string(),
            "online (no status shared)"
        );
        assert_eq!(
            SessionPresence::new(Some(" \t ")).unwrap(),
            SessionPresence::default()
        );
        assert!(SessionPresence::new(Some(&"x".repeat(MAX_PRESENCE_BYTES))).is_ok());
        assert!(SessionPresence::new(Some(&"x".repeat(MAX_PRESENCE_BYTES + 1))).is_err());

        for unsafe_status in [
            "line one\nline two",
            "tab\tseparated",
            "escape\u{001b}[31m",
            "spoof\u{202e}txt",
            "line\u{2028}separator",
        ] {
            assert!(
                SessionPresence::new(Some(unsafe_status)).is_err(),
                "accepted unsafe presence {unsafe_status:?}"
            );
        }

        let malformed_challenge = Challenge {
            magic: PROTOCOL_MAGIC.into(),
            version: PROTOCOL_VERSION,
            server_actor: actor("alice"),
            server_nonce: [0; NONCE_BYTES],
            server_presence: SessionPresence(Some(" not canonical".into())),
            server_identity: None,
            server_key_agreement: None,
        };
        assert!(validate_challenge(&malformed_challenge).is_err());
    }

    #[test]
    fn handshake_frames_reject_unknown_fields() {
        let challenge = Challenge {
            magic: PROTOCOL_MAGIC.into(),
            version: PROTOCOL_VERSION,
            server_actor: actor("alice"),
            server_nonce: [0; NONCE_BYTES],
            server_presence: SessionPresence::default(),
            server_identity: None,
            server_key_agreement: None,
        };
        let mut encoded = serde_json::to_value(challenge).unwrap();
        encoded
            .as_object_mut()
            .unwrap()
            .insert("unexpected".into(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<Challenge>(encoded).is_err());
    }

    #[test]
    fn both_session_presence_values_are_bound_to_authentication_transcript() {
        let temp = TempDir::new();
        let secret_path = temp.file("secret");
        let secret = b"0123456789abcdef0123456789abcdef";
        write_secret(&secret_path, secret);
        let server_path = temp.file("alice.aetherc");
        let base = graph("fn run() {}");
        let mut alice = GraphReplica::from_graph(actor("alice"), &base);
        let bob = alice.fork(actor("bob")).unwrap();
        alice.save(&server_path).unwrap();
        let before = std::fs::read(&server_path).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_bundle = server_path.clone();
        let host = thread::spawn(move || {
            let mut replica = GraphReplica::load(&server_bundle).unwrap();
            let (stream, _) = listener.accept().unwrap();
            handle_peer(
                &mut replica,
                stream,
                secret,
                &server_bundle,
                &SessionPresence::default(),
                None,
            )
        });

        let mut stream = TcpStream::connect(address).unwrap();
        configure_stream(&stream).unwrap();
        let challenge: Challenge = read_frame(&mut stream, MAX_HANDSHAKE_BYTES).unwrap();
        let client_nonce = random_nonce().unwrap();
        let signed_presence = SessionPresence::new(Some("reviewing")).unwrap();
        let altered_presence = SessionPresence::new(Some("approved")).unwrap();
        let transcript = transcript_bytes(
            &challenge,
            bob.actor(),
            &client_nonce,
            bob.version(),
            &signed_presence,
            &None,
            &None,
        )
        .unwrap();
        let altered_server_challenge = Challenge {
            server_presence: SessionPresence::new(Some("server is away")).unwrap(),
            ..challenge.clone()
        };
        assert_ne!(
            transcript,
            transcript_bytes(
                &altered_server_challenge,
                bob.actor(),
                &client_nonce,
                bob.version(),
                &signed_presence,
                &None,
                &None,
            )
            .unwrap()
        );
        let proof = compute_mac(secret, b"client-auth", &transcript).unwrap();
        write_frame(
            &mut stream,
            &AuthRequest {
                client_actor: bob.actor().clone(),
                client_nonce,
                client_version: bob.version().clone(),
                client_presence: altered_presence,
                client_identity: None,
                client_key_agreement: None,
                identity_proof: None,
                proof,
            },
            MAX_HANDSHAKE_BYTES,
        )
        .unwrap();
        let response: AuthResponse = read_frame(&mut stream, MAX_HANDSHAKE_BYTES).unwrap();
        assert!(!response.accepted);
        assert_eq!(response.error.as_deref(), Some("authentication failed"));
        assert_eq!(
            host.join().unwrap().unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        assert_eq!(std::fs::read(&server_path).unwrap(), before);
    }

    #[test]
    fn identity_and_key_agreement_are_bound_to_authentication_transcript() {
        let challenge = Challenge {
            magic: PROTOCOL_MAGIC.into(),
            version: PROTOCOL_VERSION,
            server_actor: actor("alice"),
            server_nonce: [1; NONCE_BYTES],
            server_presence: SessionPresence::default(),
            server_identity: Some([2; PUBLIC_KEY_BYTES]),
            server_key_agreement: Some([3; KEY_AGREEMENT_BYTES]),
        };
        let client_actor = actor("bob");
        let client_nonce = [4; NONCE_BYTES];
        let version = VersionVector::default();
        let presence = SessionPresence::default();
        let client_identity = Some([5; PUBLIC_KEY_BYTES]);
        let client_key_agreement = Some([6; KEY_AGREEMENT_BYTES]);
        let transcript = transcript_bytes(
            &challenge,
            &client_actor,
            &client_nonce,
            &version,
            &presence,
            &client_identity,
            &client_key_agreement,
        )
        .unwrap();

        let mut altered_challenge = challenge.clone();
        altered_challenge.server_identity = Some([7; PUBLIC_KEY_BYTES]);
        assert_ne!(
            transcript,
            transcript_bytes(
                &altered_challenge,
                &client_actor,
                &client_nonce,
                &version,
                &presence,
                &client_identity,
                &client_key_agreement,
            )
            .unwrap()
        );
        let mut altered_challenge = challenge.clone();
        altered_challenge.server_key_agreement = Some([8; KEY_AGREEMENT_BYTES]);
        assert_ne!(
            transcript,
            transcript_bytes(
                &altered_challenge,
                &client_actor,
                &client_nonce,
                &version,
                &presence,
                &client_identity,
                &client_key_agreement,
            )
            .unwrap()
        );
        assert_ne!(
            transcript,
            transcript_bytes(
                &challenge,
                &client_actor,
                &client_nonce,
                &version,
                &presence,
                &Some([9; PUBLIC_KEY_BYTES]),
                &client_key_agreement,
            )
            .unwrap()
        );
        assert_ne!(
            transcript,
            transcript_bytes(
                &challenge,
                &client_actor,
                &client_nonce,
                &version,
                &presence,
                &client_identity,
                &Some([10; KEY_AGREEMENT_BYTES]),
            )
            .unwrap()
        );
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
        let mut alice = GraphReplica::from_graph(actor("alice"), &graph("fn run() {}"));
        let bob = alice.fork(actor("bob")).unwrap();
        alice.save(&server_path).unwrap();
        bob.save(&client_path).unwrap();
        let before = std::fs::read(&server_path).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_bundle = server_path.clone();
        let host = thread::spawn(move || {
            serve_listener(
                &server_bundle,
                listener,
                &server_secret,
                once_listener_options(),
            )
            .unwrap()
        });
        assert!(join(
            &client_path,
            &address.to_string(),
            &client_secret,
            None,
            None,
            None,
            None
        )
        .is_err());
        host.join().unwrap();
        assert_eq!(std::fs::read(&server_path).unwrap(), before);
    }

    #[test]
    fn uninvited_actor_with_group_secret_is_rejected_without_host_mutation() {
        let temp = TempDir::new();
        let secret = temp.file("secret");
        write_secret(&secret, b"0123456789abcdef0123456789abcdef");
        let server_path = temp.file("alice.aetherc");
        let client_path = temp.file("mallory.aetherc");
        let base = graph("fn run() {}");
        let alice = GraphReplica::from_graph(actor("alice"), &base);
        let mut mallory = GraphReplica::from_graph(actor("mallory"), &base);
        mallory.add_member(actor("alice")).unwrap();
        alice.save(&server_path).unwrap();
        mallory.save(&client_path).unwrap();
        let before = std::fs::read(&server_path).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_bundle = server_path.clone();
        let server_secret = secret.clone();
        let host = thread::spawn(move || {
            serve_listener(
                &server_bundle,
                listener,
                &server_secret,
                once_listener_options(),
            )
            .unwrap()
        });
        let error = join(
            &client_path,
            &address.to_string(),
            &secret,
            None,
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("not an active collaboration member"),
            "{error}"
        );
        host.join().unwrap();
        assert_eq!(std::fs::read(&server_path).unwrap(), before);
    }

    #[test]
    fn authenticated_peer_cannot_remove_session_actor_during_sync() {
        let temp = TempDir::new();
        let secret_path = temp.file("secret");
        let secret = b"0123456789abcdef0123456789abcdef";
        write_secret(&secret_path, secret);
        let server_path = temp.file("alice.aetherc");
        let base = graph("fn run() {}");
        let mut alice = GraphReplica::from_graph(actor("alice"), &base);
        let mut bob = alice.fork(actor("bob")).unwrap();
        alice.save(&server_path).unwrap();
        bob.remove_member(&actor("alice")).unwrap();
        let malicious_delta = bob.delta_since(alice.version()).unwrap();
        let before = std::fs::read(&server_path).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_bundle = server_path.clone();
        let server_secret = secret_path.clone();
        let host = thread::spawn(move || {
            serve_listener(
                &server_bundle,
                listener,
                &server_secret,
                once_listener_options(),
            )
            .unwrap()
        });

        let mut stream = TcpStream::connect(address).unwrap();
        configure_stream(&stream).unwrap();
        let challenge: Challenge = read_frame(&mut stream, MAX_HANDSHAKE_BYTES).unwrap();
        let client_nonce = random_nonce().unwrap();
        let client_presence = SessionPresence::default();
        let transcript = transcript_bytes(
            &challenge,
            bob.actor(),
            &client_nonce,
            bob.version(),
            &client_presence,
            &None,
            &None,
        )
        .unwrap();
        let proof = compute_mac(secret, b"client-auth", &transcript).unwrap();
        write_frame(
            &mut stream,
            &AuthRequest {
                client_actor: bob.actor().clone(),
                client_nonce,
                client_version: bob.version().clone(),
                client_presence,
                client_identity: None,
                client_key_agreement: None,
                identity_proof: None,
                proof,
            },
            MAX_HANDSHAKE_BYTES,
        )
        .unwrap();
        let response: AuthResponse = read_frame(&mut stream, MAX_HANDSHAKE_BYTES).unwrap();
        assert!(response.accepted);
        verify_mac(secret, b"server-auth", &transcript, &response.proof).unwrap();
        let session_key = compute_mac(secret, b"session-key", &transcript).unwrap();
        let server_payload = read_signed(&mut stream, &session_key, b"server", 1).unwrap();
        assert!(matches!(server_payload, SyncPayload::ServerDelta { .. }));
        write_signed(
            &mut stream,
            &session_key,
            b"client",
            1,
            SyncPayload::ClientDelta {
                delta: malicious_delta,
            },
        )
        .unwrap();
        let rejection = read_signed(&mut stream, &session_key, b"server", 2).unwrap();
        let SyncPayload::Error { message } = rejection else {
            panic!("expected session-membership rejection");
        };
        assert!(message.contains("cannot remove authenticated session actors"));

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
        let current_path = temp.file("charlie-current.aetherc");
        let base = graph("fn run() {}");
        let mut alice = GraphReplica::from_graph(actor("alice"), &base);
        let mut charlie = alice.fork(actor("charlie")).unwrap();
        charlie.save(&stale_path).unwrap();
        alice.sync_graph(&graph("fn run() { first(); }")).unwrap();
        alice.sync_graph(&graph("fn run() { second(); }")).unwrap();
        charlie
            .apply_delta(&alice.delta_since(charlie.version()).unwrap())
            .unwrap();
        charlie.save(&current_path).unwrap();
        alice
            .acknowledge(actor("charlie"), charlie.version().clone())
            .unwrap();
        assert!(alice.compact_acknowledged().unwrap().removed_operations > 0);
        alice.save(&server_path).unwrap();
        let before = std::fs::read(&server_path).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_bundle = server_path.clone();
        let server_secret = secret.clone();
        let host = thread::spawn(move || {
            serve_listener(
                &server_bundle,
                listener,
                &server_secret,
                once_listener_options(),
            )
            .unwrap()
        });
        let error = join(
            &stale_path,
            &address.to_string(),
            &secret,
            Some(&stale_path),
            None,
            None,
            None,
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
