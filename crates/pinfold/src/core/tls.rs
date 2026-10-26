//! The CONNECT SNI check. rustls reads the ClientHello; the bytes it read
//! are kept, to be forwarded unchanged.

use std::io::{self, Read};

use rustls::server::Acceptor;

/// The most that is buffered before the ClientHello is complete. Real ones
/// are under 2 KiB; the cap only stops a client growing the buffer without
/// bound.
const MAX_HELLO: usize = 16 * 1024;

const MALFORMED: &str = "malformed clienthello";

/// Read until rustls has one whole ClientHello, and check that its SNI
/// names `host`. It returns every byte read, the ClientHello and anything
/// the client sent after it in the same reads, to forward to the server
/// unchanged; the error is the refusal's log reason.
pub fn read_client_hello(reader: &mut impl Read, host: &str) -> Result<Vec<u8>, &'static str> {
    let mut tee = Tee {
        reader,
        bytes: Vec::new(),
    };
    let mut acceptor = Acceptor::default();
    let accepted = loop {
        match acceptor.read_tls(&mut tee) {
            Ok(0) => return Err(MALFORMED),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(MALFORMED),
        }
        match acceptor.accept() {
            Ok(Some(accepted)) => break accepted,
            Ok(None) => {}
            Err(_) => return Err(MALFORMED),
        }
    };
    match accepted.client_hello().server_name() {
        None => Err("sni missing"),
        Some(sni) if !sni.eq_ignore_ascii_case(host) => Err("sni mismatch"),
        Some(_) => Ok(tee.bytes),
    }
}

/// A reader that keeps every byte it passes on, up to [`MAX_HELLO`].
struct Tee<'a, R> {
    reader: &'a mut R,
    bytes: Vec<u8>,
}

impl<R: Read> Read for Tee<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let room = MAX_HELLO - self.bytes.len();
        if room == 0 {
            return Err(io::Error::other("clienthello too large"));
        }
        let end = buf.len().min(room);
        let read = self.reader.read(&mut buf[..end])?;
        self.bytes.extend_from_slice(&buf[..read]);
        Ok(read)
    }
}
