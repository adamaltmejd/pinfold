//! The CONNECT SNI check's ClientHello parser. Only the SNI is read; the
//! exact bytes are returned to be forwarded unchanged.

use std::io::Read;

/// The most of one ClientHello that will be buffered. Real ones are under
/// 2 KiB; the cap only stops a client growing the buffer without bound.
const MAX_HELLO: usize = 16 * 1024;

const MALFORMED: &str = "malformed clienthello";

/// A ClientHello read from the client.
pub struct ClientHello {
    /// The TLS records read, to forward to the server unchanged.
    pub bytes: Vec<u8>,
    /// The `host_name` from the SNI extension, when the client sent one.
    pub sni: Option<String>,
}

/// Read one ClientHello, ending exactly after the handshake message's last
/// record, so the tunnel starts where the ClientHello ends. The error is the
/// refusal's log reason.
pub fn read_client_hello(reader: &mut impl Read) -> Result<ClientHello, &'static str> {
    let mut bytes = Vec::new();
    let mut handshake = Vec::new();
    loop {
        let mut record = [0u8; 5];
        reader
            .read_exact(&mut record)
            .map_err(|_| "clienthello read failed")?;
        if record[0] != 0x16 {
            return Err("not tls");
        }
        let length = usize::from(u16::from_be_bytes([record[3], record[4]]));
        if length == 0 {
            return Err(MALFORMED);
        }
        if bytes.len() + 5 + length > MAX_HELLO {
            return Err("clienthello too large");
        }
        bytes.extend_from_slice(&record);
        let start = bytes.len();
        bytes.resize(start + length, 0);
        reader
            .read_exact(&mut bytes[start..])
            .map_err(|_| "clienthello read failed")?;
        handshake.extend_from_slice(&bytes[start..]);
        if handshake.len() >= 4 && handshake.len() >= 4 + handshake_length(&handshake) {
            break;
        }
    }
    if handshake[0] != 0x01 {
        return Err("not a clienthello");
    }
    let length = handshake_length(&handshake);
    let sni = parse_sni(&handshake[4..4 + length])?;
    Ok(ClientHello { bytes, sni })
}

/// The three-byte handshake message length.
fn handshake_length(handshake: &[u8]) -> usize {
    (usize::from(handshake[1]) << 16) | (usize::from(handshake[2]) << 8) | usize::from(handshake[3])
}

/// The SNI host name, when the ClientHello carries one.
fn parse_sni(hello: &[u8]) -> Result<Option<String>, &'static str> {
    let mut cursor = Cursor(hello);
    cursor.take(34)?; // legacy_version, random
    cursor.vec_u8()?; // session id
    cursor.vec_u16()?; // cipher suites
    cursor.vec_u8()?; // compression methods
    if cursor.is_empty() {
        return Ok(None);
    }
    let mut extensions = Cursor(cursor.vec_u16()?);
    while !extensions.is_empty() {
        let kind = extensions.u16()?;
        let data = extensions.vec_u16()?;
        if kind == 0 {
            return server_name(data);
        }
    }
    Ok(None)
}

/// The `host_name` from one server_name extension.
fn server_name(data: &[u8]) -> Result<Option<String>, &'static str> {
    let mut cursor = Cursor(data);
    let mut list = Cursor(cursor.vec_u16()?);
    while !list.is_empty() {
        let kind = list.u8()?;
        let name = list.vec_u16()?;
        if kind == 0 {
            let name = std::str::from_utf8(name).map_err(|_| MALFORMED)?;
            return Ok(Some(name.to_string()));
        }
    }
    Ok(None)
}

/// A bounds-checked reader over one slice.
struct Cursor<'a>(&'a [u8]);

impl<'a> Cursor<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], &'static str> {
        let (taken, rest) = self.0.split_at_checked(length).ok_or(MALFORMED)?;
        self.0 = rest;
        Ok(taken)
    }

    fn u8(&mut self) -> Result<u8, &'static str> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, &'static str> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    /// A vector with a one-byte length prefix.
    fn vec_u8(&mut self) -> Result<&'a [u8], &'static str> {
        let length = usize::from(self.u8()?);
        self.take(length)
    }

    /// A vector with a two-byte length prefix.
    fn vec_u16(&mut self) -> Result<&'a [u8], &'static str> {
        let length = usize::from(self.u16()?);
        self.take(length)
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
