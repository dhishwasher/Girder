#[allow(dead_code)]
#[path = "../src/project/license.rs"]
mod license;

use ring::signature::{Ed25519KeyPair, KeyPair};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const PRODUCTION_PUBLIC_KEY: [u8; 32] = [
    113, 93, 68, 144, 174, 49, 174, 41, 59, 13, 191, 40, 60, 66, 143, 168, 131, 224, 49, 185, 98,
    213, 185, 210, 5, 112, 139, 39, 37, 191, 208, 116,
];

fn main() {
    if let Err(error) = run(std::env::args().skip(1)) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let private_key = args
        .next()
        .ok_or_else(|| usage("missing private key path"))?;
    let tier = args.next().ok_or_else(|| usage("missing tier"))?;
    if args.next().is_some() {
        return Err(usage("too many arguments"));
    }
    if !matches!(tier.as_str(), "free" | "paid") {
        return Err("tier must be `free` or `paid`".into());
    }

    let private = fs::read(&private_key).map_err(|error| {
        format!(
            "could not read {}: {error}",
            Path::new(&private_key).display()
        )
    })?;
    let pair = Ed25519KeyPair::from_pkcs8_maybe_unchecked(&private)
        .map_err(|_| "private key must be an Ed25519 PKCS#8 DER file".to_string())?;
    if pair.public_key().as_ref() != PRODUCTION_PUBLIC_KEY {
        return Err("private key does not match Girder's compiled-in public key".into());
    }

    let issued_on = utc_date(SystemTime::now())?;
    let unsigned = license::unsigned_key(&issued_on, &tier).map_err(|error| error.to_string())?;
    let signature = encode_signature(pair.sign(unsigned.as_bytes()).as_ref());
    println!("{unsigned}.{signature}");
    Ok(())
}

fn encode_signature(signature: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(signature.len() * 2);
    for byte in signature {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

fn usage(reason: &str) -> String {
    format!("{reason}\nusage: license_keygen <private-key.pk8> <free|paid>")
}

fn utc_date(now: SystemTime) -> Result<String, String> {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before the Unix epoch".to_string())?
        .as_secs();
    let (year, month, day) = civil_from_days((seconds / 86_400) as i64);
    Ok(format!("{year:04}-{month:02}-{day:02}"))
}

// Howard Hinnant's public-domain civil calendar conversion, adapted to Rust.
fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let days = days_since_epoch + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn utc_issue_date_is_derived_without_network_or_locale() {
        assert_eq!(utc_date(UNIX_EPOCH).unwrap(), "1970-01-01");
        assert_eq!(
            utc_date(UNIX_EPOCH + Duration::from_secs(10_957 * 86_400)).unwrap(),
            "2000-01-01"
        );
    }
}
