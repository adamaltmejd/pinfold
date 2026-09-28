//! pinfold's core: box specs, profiles, runtimes and the box lifecycle.

pub mod artifacts;
pub mod r#box;
pub mod clean;
pub mod image;
pub mod login;
pub mod network;
pub mod plan;
pub mod profile;
pub mod proxy;
pub mod runtime;
pub mod tls;

use sha2::{Digest, Sha256};

/// The sha256 of `bytes` as lowercase hex. Ids and cache paths on disk are
/// made of it.
pub(crate) fn sha256_hex(bytes: impl AsRef<[u8]>) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Seconds since the Unix epoch; 0 for a clock before it.
pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// The RFC 3339 UTC time for a Unix timestamp in seconds. `box list`'s
/// `created` and the egress log's `time` share this one shape.
pub(crate) fn rfc3339(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let time = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (time / 3_600, time % 3_600 / 60, time % 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Days since 1970-01-01 to a proleptic Gregorian date, after Howard
/// Hinnant's `civil_from_days` (public domain).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}
