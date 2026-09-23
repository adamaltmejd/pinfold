//! The CONNECT SNI check's ClientHello parser.
//!
//! Small and ours: the proxy carries no TLS dependency. Only the SNI is
//! read; the exact bytes are returned to be forwarded unchanged.

use std::io::Read;

/// The most of one ClientHello that will be buffered. Real ones are under
/// 2 KiB; the cap only stops a client growing the buffer without bound.
const MAX_HELLO: usize = 16 * 1024;

/// A ClientHello read from the client.
pub struct ClientHello {
    /// The TLS records read, to forward to the server unchanged.
    pub bytes: Vec<u8>,
    /// The `host_name` from the SNI extension, when the client sent one.
    pub sni: Option<String>,
}

/// Why a ClientHello could not be read.
#[derive(Debug)]
pub enum HelloError {
    /// The client did not start with a TLS handshake record.
    NotTls,
    /// The handshake message is not a ClientHello.
    NotClientHello,
    /// The ClientHello is malformed or holds no valid SNI name.
    Malformed,
    /// The ClientHello is larger than the cap.
    TooLarge,
    /// The read failed, timed out or hit EOF.
    Read,
}

impl HelloError {
    /// The log reason for a refusal.
    pub fn reason(&self) -> &'static str {
        match self {
            HelloError::NotTls => "not tls",
            HelloError::NotClientHello => "not a clienthello",
            HelloError::Malformed => "malformed clienthello",
            HelloError::TooLarge => "clienthello too large",
            HelloError::Read => "clienthello read failed",
        }
    }
}

/// Read one ClientHello, ending exactly after the handshake message's last
/// record, so the tunnel starts where the ClientHello ends.
pub fn read_client_hello(reader: &mut impl Read) -> Result<ClientHello, HelloError> {
    let mut bytes = Vec::new();
    let mut handshake = Vec::new();
    loop {
        let mut record = [0u8; 5];
        read_exact(reader, &mut record)?;
        if record[0] != 0x16 {
            return Err(HelloError::NotTls);
        }
        let length = usize::from(u16::from_be_bytes([record[3], record[4]]));
        if length == 0 {
            return Err(HelloError::Malformed);
        }
        if bytes.len() + 5 + length > MAX_HELLO {
            return Err(HelloError::TooLarge);
        }
        bytes.extend_from_slice(&record);
        let start = bytes.len();
        bytes.resize(start + length, 0);
        read_exact(reader, &mut bytes[start..])?;
        handshake.extend_from_slice(&bytes[start..]);
        if handshake.len() >= 4 && handshake.len() >= 4 + handshake_length(&handshake) {
            break;
        }
    }
    if handshake[0] != 0x01 {
        return Err(HelloError::NotClientHello);
    }
    let length = handshake_length(&handshake);
    let sni = parse_sni(&handshake[4..4 + length])?;
    Ok(ClientHello { bytes, sni })
}

fn read_exact(reader: &mut impl Read, buffer: &mut [u8]) -> Result<(), HelloError> {
    reader.read_exact(buffer).map_err(|_| HelloError::Read)
}

/// The three-byte handshake message length.
fn handshake_length(handshake: &[u8]) -> usize {
    (usize::from(handshake[1]) << 16) | (usize::from(handshake[2]) << 8) | usize::from(handshake[3])
}

/// The SNI host name, when the ClientHello carries one.
fn parse_sni(hello: &[u8]) -> Result<Option<String>, HelloError> {
    let mut cursor = Cursor::new(hello);
    cursor.take(2)?; // legacy_version
    cursor.take(32)?; // random
    let session = cursor.length_u8()?;
    cursor.take(session)?;
    let suites = cursor.length_u16()?;
    cursor.take(suites)?;
    let methods = cursor.length_u8()?;
    cursor.take(methods)?;
    if cursor.is_empty() {
        return Ok(None);
    }
    let extensions = cursor.length_u16()?;
    let mut extensions = Cursor::new(cursor.take(extensions)?);
    while !extensions.is_empty() {
        let kind = extensions.u16()?;
        let length = extensions.length_u16()?;
        let data = extensions.take(length)?;
        if kind == 0 {
            return server_name(data);
        }
    }
    Ok(None)
}

/// The `host_name` from one server_name extension.
fn server_name(data: &[u8]) -> Result<Option<String>, HelloError> {
    let mut cursor = Cursor::new(data);
    let list = cursor.length_u16()?;
    let mut list = Cursor::new(cursor.take(list)?);
    while !list.is_empty() {
        let kind = list.u8()?;
        let length = list.length_u16()?;
        let name = list.take(length)?;
        if kind == 0 {
            let name = std::str::from_utf8(name).map_err(|_| HelloError::Malformed)?;
            return Ok(Some(name.to_string()));
        }
    }
    Ok(None)
}

/// A bounds-checked reader over one slice.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Cursor<'a> {
        Cursor { bytes, at: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], HelloError> {
        let end = self.at.checked_add(length).ok_or(HelloError::Malformed)?;
        if end > self.bytes.len() {
            return Err(HelloError::Malformed);
        }
        let taken = &self.bytes[self.at..end];
        self.at = end;
        Ok(taken)
    }

    fn u8(&mut self) -> Result<u8, HelloError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, HelloError> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn length_u8(&mut self) -> Result<usize, HelloError> {
        Ok(usize::from(self.u8()?))
    }

    fn length_u16(&mut self) -> Result<usize, HelloError> {
        Ok(usize::from(self.u16()?))
    }

    fn is_empty(&self) -> bool {
        self.at >= self.bytes.len()
    }
}
