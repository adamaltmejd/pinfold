//! A box's CA and its one leaf for the routed hosts (Egress proxy,
//! Interception). ring makes the keys and signatures; the two certificates
//! are a fixed DER template. Neither private key leaves this process's
//! memory, and the CA's is dropped once the leaf is signed.

use std::io;

use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

pub use crate::init::BUNDLE;

/// The variables a box with a routed host gets, each naming [`BUNDLE`]:
/// OpenSSL's (Python, Ruby, PHP, curl), Node's and Bun's, and Python
/// requests'.
pub const TRUST_ENV: [&str; 3] = ["SSL_CERT_FILE", "NODE_EXTRA_CA_CERTS", "REQUESTS_CA_BUNDLE"];

/// Back-dated so a guest clock behind the host's still accepts it.
const BEFORE: u64 = 24 * 60 * 60;
/// Some clients refuse a leaf valid longer than 398 days.
const LIFETIME: u64 = 397 * 24 * 60 * 60;

/// The box CA and its leaf. No `Debug`: it holds the leaf's key.
pub struct Issued {
    /// The CA certificate, PEM: the only part the box gets.
    pub ca_pem: String,
    pub leaf: CertificateDer<'static>,
    pub key: PrivateKeyDer<'static>,
}

fn fail<E>(_: E) -> io::Error {
    io::Error::other("box CA: key generation failed")
}

/// Issue a CA named for `label` and one leaf naming every host in `hosts`,
/// valid from a day before `now` for 397 days.
pub fn issue(hosts: &[String], label: &str, now: u64) -> io::Result<Issued> {
    let rng = SystemRandom::new();
    let generate = || -> io::Result<EcdsaKeyPair> {
        let pkcs8 =
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).map_err(fail)?;
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
            .map_err(fail)
    };
    let ca_key = generate()?;
    let leaf_pkcs8 =
        EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).map_err(fail)?;
    let leaf_key =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, leaf_pkcs8.as_ref(), &rng)
            .map_err(fail)?;
    let validity = seq(&[
        &time(now.saturating_sub(BEFORE)),
        &time(now.saturating_add(LIFETIME)),
    ]);
    let ca_name = name(&format!("pinfold box CA {label}"));
    let ca_id = key_id(ca_key.public_key().as_ref());
    let ca_extensions = [
        // CA:TRUE, pathLen 0.
        extension(BASIC_CONSTRAINTS, true, &seq(&[&[1, 1, 0xff], &[2, 1, 0]])),
        // keyCertSign and cRLSign.
        extension(KEY_USAGE, true, &[3, 2, 1, 0x06]),
        extension(SUBJECT_KEY_ID, false, &tlv(0x04, &ca_id)),
    ];
    let ca_tbs = tbs(
        &serial(&rng)?,
        &ca_name,
        &validity,
        &ca_name,
        ca_key.public_key().as_ref(),
        &ca_extensions,
    );
    let ca = sign(&ca_key, &rng, ca_tbs)?;

    let names: Vec<Vec<u8>> = hosts
        .iter()
        .map(|host| tlv(0x82, host.as_bytes()))
        .collect();
    let names: Vec<&[u8]> = names.iter().map(Vec::as_slice).collect();
    let leaf_extensions = [
        // CA:FALSE, the default, encoded as an empty sequence.
        extension(BASIC_CONSTRAINTS, true, &seq(&[])),
        // digitalSignature.
        extension(KEY_USAGE, true, &[3, 2, 7, 0x80]),
        extension(EXTENDED_KEY_USAGE, false, &seq(&[&tlv(0x06, SERVER_AUTH)])),
        extension(SUBJECT_ALT_NAME, false, &seq(&names)),
        extension(
            SUBJECT_KEY_ID,
            false,
            &tlv(0x04, &key_id(leaf_key.public_key().as_ref())),
        ),
        extension(AUTHORITY_KEY_ID, false, &seq(&[&tlv(0x80, &ca_id)])),
    ];
    let leaf_tbs = tbs(
        &serial(&rng)?,
        &ca_name,
        &validity,
        &name(hosts.first().map_or("pinfold", String::as_str)),
        leaf_key.public_key().as_ref(),
        &leaf_extensions,
    );
    let leaf = sign(&ca_key, &rng, leaf_tbs)?;
    Ok(Issued {
        ca_pem: pem(&ca),
        leaf: CertificateDer::from(leaf),
        key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_pkcs8.as_ref().to_vec())),
    })
}

// Each OID's content octets.
const ECDSA_WITH_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02];
const EC_PUBLIC_KEY: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const PRIME256V1: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
const COMMON_NAME: &[u8] = &[0x55, 0x04, 0x03];
const SUBJECT_KEY_ID: &[u8] = &[0x55, 0x1d, 0x0e];
const KEY_USAGE: &[u8] = &[0x55, 0x1d, 0x0f];
const SUBJECT_ALT_NAME: &[u8] = &[0x55, 0x1d, 0x11];
const BASIC_CONSTRAINTS: &[u8] = &[0x55, 0x1d, 0x13];
const AUTHORITY_KEY_ID: &[u8] = &[0x55, 0x1d, 0x23];
const EXTENDED_KEY_USAGE: &[u8] = &[0x55, 0x1d, 0x25];
const SERVER_AUTH: &[u8] = &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01];

/// One DER element, its length in the short or long form.
fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    match content.len() {
        length @ 0..=0x7f => out.push(length as u8),
        length => {
            let bytes = length.to_be_bytes();
            let skip = bytes.iter().take_while(|byte| **byte == 0).count();
            out.push(0x80 | (bytes.len() - skip) as u8);
            out.extend_from_slice(&bytes[skip..]);
        }
    }
    out.extend_from_slice(content);
    out
}

fn seq(parts: &[&[u8]]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}

fn algorithm() -> Vec<u8> {
    seq(&[&tlv(0x06, ECDSA_WITH_SHA256)])
}

/// A Name holding one CN.
fn name(common: &str) -> Vec<u8> {
    let attribute = seq(&[&tlv(0x06, COMMON_NAME), &tlv(0x0c, common.as_bytes())]);
    seq(&[&tlv(0x31, &attribute)])
}

/// UTCTime through 2049, GeneralizedTime after (RFC 5280, 4.1.2.5).
fn time(seconds: u64) -> Vec<u8> {
    let seconds = seconds as i64;
    let (year, month, day) = crate::core::civil_from_days(seconds.div_euclid(86_400));
    let time = seconds.rem_euclid(86_400);
    let rest = format!(
        "{month:02}{day:02}{:02}{:02}{:02}Z",
        time / 3_600,
        time % 3_600 / 60,
        time % 60
    );
    if year < 2050 {
        tlv(0x17, format!("{:02}{rest}", year % 100).as_bytes())
    } else {
        tlv(0x18, format!("{year:04}{rest}").as_bytes())
    }
}

/// 16 random bytes, the top bit cleared and the next set, so the INTEGER
/// is positive and needs no leading zero.
fn serial(rng: &SystemRandom) -> io::Result<Vec<u8>> {
    let mut bytes = [0u8; 16];
    rng.fill(&mut bytes)
        .map_err(|_| io::Error::other("box CA: no randomness"))?;
    bytes[0] = (bytes[0] & 0x7f) | 0x40;
    Ok(tlv(0x02, &bytes))
}

/// The first 20 bytes of the key's SHA-256 (RFC 7093, method 1).
fn key_id(public: &[u8]) -> Vec<u8> {
    ring::digest::digest(&ring::digest::SHA256, public).as_ref()[..20].to_vec()
}

fn extension(oid: &[u8], critical: bool, value: &[u8]) -> Vec<u8> {
    let critical: &[u8] = if critical { &[1, 1, 0xff] } else { &[] };
    seq(&[&tlv(0x06, oid), critical, &tlv(0x04, value)])
}

fn tbs(
    serial: &[u8],
    issuer: &[u8],
    validity: &[u8],
    subject: &[u8],
    public: &[u8],
    extensions: &[Vec<u8>],
) -> Vec<u8> {
    let key = seq(&[
        &seq(&[&tlv(0x06, EC_PUBLIC_KEY), &tlv(0x06, PRIME256V1)]),
        &tlv(0x03, &[&[0u8][..], public].concat()),
    ]);
    let extensions: Vec<&[u8]> = extensions.iter().map(Vec::as_slice).collect();
    seq(&[
        // [0] EXPLICIT version v3.
        &[0xa0, 3, 2, 1, 2],
        serial,
        &algorithm(),
        issuer,
        validity,
        subject,
        &key,
        &tlv(0xa3, &seq(&extensions)),
    ])
}

fn sign(key: &EcdsaKeyPair, rng: &SystemRandom, tbs: Vec<u8>) -> io::Result<Vec<u8>> {
    let signature = key
        .sign(rng, &tbs)
        .map_err(|_| io::Error::other("box CA: signing failed"))?;
    Ok(seq(&[
        &tbs,
        &algorithm(),
        &tlv(0x03, &[&[0u8][..], signature.as_ref()].concat()),
    ]))
}

fn pem(der: &[u8]) -> String {
    let base64 = crate::core::plan::base64(der);
    let mut out = String::from("-----BEGIN CERTIFICATE-----\n");
    for line in base64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).expect("base64 is ASCII"));
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}
