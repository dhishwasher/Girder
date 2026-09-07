use ring::signature::{UnparsedPublicKey, ED25519};
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;

const KEY_PREFIX: &str = "girder-v1";
const PUBLIC_KEY: [u8; 32] = [
    113, 93, 68, 144, 174, 49, 174, 41, 59, 13, 191, 40, 60, 66, 143, 168, 131, 224, 49, 185, 98,
    213, 185, 210, 5, 112, 139, 39, 37, 191, 208, 116,
];

// Integration tests exercise paid commands through the real debug binary. The
// corresponding test private key was discarded; release binaries do not
// compile this public key and cannot accept the signed test token.
#[cfg(debug_assertions)]
const TEST_PUBLIC_KEY: [u8; 32] = [
    238, 120, 27, 233, 235, 199, 95, 108, 249, 72, 106, 23, 150, 253, 220, 77, 102, 213, 196, 138,
    164, 55, 173, 138, 132, 196, 192, 84, 226, 112, 198, 36,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Tier {
    Free,
    Paid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LicenseRejection {
    Malformed,
    BadSignature,
    UnknownTier,
}

impl fmt::Display for LicenseRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Malformed => "malformed license key",
            Self::BadSignature => "license key has a bad signature",
            Self::UnknownTier => "license key names an unknown tier",
        })
    }
}

#[derive(Debug)]
pub(crate) enum LicenseError {
    Rejected(LicenseRejection),
    Read { path: PathBuf, source: io::Error },
}

impl fmt::Display for LicenseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(rejection) => rejection.fmt(formatter),
            Self::Read { path, source } => {
                write!(
                    formatter,
                    "could not read license key at {}: {source}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for LicenseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Rejected(_) => None,
            Self::Read { source, .. } => Some(source),
        }
    }
}

pub(crate) fn current_tier() -> Result<Tier, LicenseError> {
    let key = match std::env::var("GIRDER_LICENSE_KEY") {
        Ok(key) => Some(key),
        Err(std::env::VarError::NotPresent) => read_key_file()?,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(LicenseError::Rejected(LicenseRejection::Malformed))
        }
    };

    key.as_deref()
        .map(verify_key)
        .transpose()
        .map(|tier| tier.unwrap_or(Tier::Free))
        .map_err(LicenseError::Rejected)
}

pub(crate) fn require_paid(tool: &str) -> io::Result<()> {
    let alternative =
        "Free alternative: `get_source` and `find_definition` still answer exact-symbol questions.";
    match current_tier() {
        Ok(Tier::Paid) => Ok(()),
        Ok(Tier::Free) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("The `{tool}` tool needs a paid Girder license. {alternative}"),
        )),
        Err(error) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "The `{tool}` tool needs a valid paid Girder license, but the configured key was rejected: {error}. {alternative}"
            ),
        )),
    }
}

pub(crate) fn verify_key(key: &str) -> Result<Tier, LicenseRejection> {
    let result = verify_key_with_public_key(key, &PUBLIC_KEY);
    #[cfg(debug_assertions)]
    if result == Err(LicenseRejection::BadSignature) {
        return verify_key_with_public_key(key, &TEST_PUBLIC_KEY);
    }
    result
}

pub(crate) fn unsigned_key(issued_on: &str, tier: &str) -> Result<String, LicenseRejection> {
    if !valid_date(issued_on) || tier.is_empty() || tier.contains('.') {
        return Err(LicenseRejection::Malformed);
    }
    Ok(format!("{KEY_PREFIX}.{issued_on}.{tier}"))
}

#[cfg(test)]
fn encode_signature(signature: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(signature.len() * 2);
    for byte in signature {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

fn verify_key_with_public_key(key: &str, public_key: &[u8]) -> Result<Tier, LicenseRejection> {
    let mut parts = key.trim().split('.');
    let prefix = parts.next().ok_or(LicenseRejection::Malformed)?;
    let issued_on = parts.next().ok_or(LicenseRejection::Malformed)?;
    let tier_name = parts.next().ok_or(LicenseRejection::Malformed)?;
    let signature = parts.next().ok_or(LicenseRejection::Malformed)?;
    if parts.next().is_some() || prefix != KEY_PREFIX || !valid_date(issued_on) {
        return Err(LicenseRejection::Malformed);
    }

    let tier = match tier_name {
        "free" => Tier::Free,
        "paid" => Tier::Paid,
        _ => return Err(LicenseRejection::UnknownTier),
    };
    let signature = decode_signature(signature)?;
    let unsigned = unsigned_key(issued_on, tier_name)?;
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(unsigned.as_bytes(), &signature)
        .map_err(|_| LicenseRejection::BadSignature)?;
    Ok(tier)
}

fn decode_signature(encoded: &str) -> Result<[u8; 64], LicenseRejection> {
    if encoded.len() != 128 || !encoded.is_ascii() {
        return Err(LicenseRejection::Malformed);
    }
    let mut signature = [0_u8; 64];
    for (index, pair) in encoded.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        signature[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Ok(signature)
}

fn hex_nibble(byte: u8) -> Result<u8, LicenseRejection> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(LicenseRejection::Malformed),
    }
}

fn valid_date(date: &str) -> bool {
    let bytes = date.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| index != 4 && index != 7 && !byte.is_ascii_digit())
    {
        return false;
    }
    let Ok(year) = date[0..4].parse::<u16>() else {
        return false;
    };
    let Ok(month) = date[5..7].parse::<u8>() else {
        return false;
    };
    let Ok(day) = date[8..10].parse::<u8>() else {
        return false;
    };
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    day > 0 && day <= days
}

fn read_key_file() -> Result<Option<String>, LicenseError> {
    let Some(path) = license_key_path() else {
        return Ok(None);
    };
    match fs::read_to_string(&path) {
        Ok(key) => Ok(Some(key.trim().to_owned())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(LicenseError::Read { path, source }),
    }
}

pub(crate) fn license_key_path() -> Option<PathBuf> {
    config_dir().map(|root| root.join("girder").join("license.key"))
}

#[cfg(target_os = "windows")]
fn config_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn config_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library").join("Application Support"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

#[cfg(not(any(unix, target_os = "windows")))]
fn config_dir() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    const DEBUG_TEST_LICENSE_KEY: &str = "girder-v1.2026-09-06.paid.c3a189213567f3aced881143c0d600df36c162252ff026ee6a6377a85959215b90ca7ea51e2eb474d3e8ca4e60b09648994a1e6513772393dcde1b4e7752bd02";

    fn signed_key(issued_on: &str, tier: &str) -> (String, Vec<u8>) {
        let rng = SystemRandom::new();
        let private = Ed25519KeyPair::generate_pkcs8(&rng).expect("generate test key");
        let pair = Ed25519KeyPair::from_pkcs8(private.as_ref()).expect("parse test key");
        let unsigned = unsigned_key(issued_on, tier).expect("valid test payload");
        let signature = encode_signature(pair.sign(unsigned.as_bytes()).as_ref());
        (
            format!("{unsigned}.{signature}"),
            pair.public_key().as_ref().to_vec(),
        )
    }

    #[test]
    fn valid_key_returns_its_tier() {
        let (key, public_key) = signed_key("2026-09-06", "paid");
        assert_eq!(
            verify_key_with_public_key(&key, &public_key),
            Ok(Tier::Paid)
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn debug_build_accepts_the_debug_test_key() {
        assert_eq!(verify_key(DEBUG_TEST_LICENSE_KEY), Ok(Tier::Paid));
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn release_build_rejects_the_debug_test_key() {
        assert_eq!(
            verify_key(DEBUG_TEST_LICENSE_KEY),
            Err(LicenseRejection::BadSignature)
        );
    }

    #[test]
    fn malformed_key_has_a_specific_rejection() {
        assert_eq!(
            verify_key("not-a-license-key"),
            Err(LicenseRejection::Malformed)
        );
    }

    #[test]
    fn tampered_key_has_a_bad_signature() {
        let (key, public_key) = signed_key("2026-09-06", "paid");
        let tampered = key.replacen(".paid.", ".free.", 1);
        assert_eq!(
            verify_key_with_public_key(&tampered, &public_key),
            Err(LicenseRejection::BadSignature)
        );
    }

    #[test]
    fn unknown_tier_has_a_specific_rejection() {
        let (key, public_key) = signed_key("2026-09-06", "enterprise");
        assert_eq!(
            verify_key_with_public_key(&key, &public_key),
            Err(LicenseRejection::UnknownTier)
        );
    }
}
