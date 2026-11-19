//! Streaming MD5 (constant memory) — extracted from logos-osm verify.rs.
use md5::{Digest, Md5};

#[derive(Default)]
pub struct Md5Stream {
    inner: Md5,
    len: u64,
}

impl Md5Stream {
    pub fn new() -> Self { Self::default() }
    pub fn update(&mut self, chunk: &[u8]) {
        self.inner.update(chunk);
        self.len += chunk.len() as u64;
    }
    pub fn len(&self) -> u64 { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    pub fn finalize(self) -> [u8; 16] { self.inner.finalize().into() }
}

pub fn md5_bytes(data: &[u8]) -> [u8; 16] {
    let mut s = Md5Stream::new();
    s.update(data);
    s.finalize()
}
