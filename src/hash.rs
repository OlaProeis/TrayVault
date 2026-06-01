//! Hand-rolled SHA-256 (FIPS 180-4) and clipboard content hashing for dedup.
//!
//! No external crypto crates — block compression, padding, and length encoding
//! are implemented in safe Rust. See `docs/technical/hashing-dedup.md`.

use crate::models::{ClipEntry, EntryKind};

// ---------------------------------------------------------------------------
// SHA-256 core (FIPS 180-4)
// ---------------------------------------------------------------------------

const H0: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

const K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

/// Incremental SHA-256 hasher.
#[derive(Clone, Debug)]
pub struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffer_len: usize,
    total_len: u64,
}

impl Sha256 {
    pub fn new() -> Self {
        Self {
            state: H0,
            buffer: [0u8; 64],
            buffer_len: 0,
            total_len: 0,
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.total_len = self.total_len.saturating_add(data.len() as u64);
        let mut offset = 0usize;

        if self.buffer_len > 0 {
            let space = 64 - self.buffer_len;
            let take = space.min(data.len());
            self.buffer[self.buffer_len..self.buffer_len + take].copy_from_slice(&data[..take]);
            self.buffer_len += take;
            offset += take;

            if self.buffer_len == 64 {
                compress_block(&mut self.state, &self.buffer);
                self.buffer_len = 0;
            }
        }

        while offset + 64 <= data.len() {
            compress_block(&mut self.state, &data[offset..offset + 64]);
            offset += 64;
        }

        if offset < data.len() {
            let remaining = data.len() - offset;
            self.buffer[..remaining].copy_from_slice(&data[offset..]);
            self.buffer_len = remaining;
        }
    }

    pub fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.total_len.wrapping_mul(8);

        // Append the 0x80 padding bit without updating `total_len`.
        if self.buffer_len == 64 {
            compress_block(&mut self.state, &self.buffer);
            self.buffer_len = 0;
        }
        self.buffer[self.buffer_len] = 0x80;
        self.buffer_len += 1;

        if self.buffer_len > 56 {
            while self.buffer_len < 64 {
                self.buffer[self.buffer_len] = 0;
                self.buffer_len += 1;
            }
            compress_block(&mut self.state, &self.buffer);
            self.buffer_len = 0;
        }

        while self.buffer_len < 56 {
            self.buffer[self.buffer_len] = 0;
            self.buffer_len += 1;
        }

        self.buffer[56..64].copy_from_slice(&bit_len.to_be_bytes());
        compress_block(&mut self.state, &self.buffer);

        let mut out = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

/// One-shot SHA-256 over raw bytes.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize()
}

fn compress_block(state: &mut [u32; 8], block: &[u8]) {
    debug_assert_eq!(block.len(), 64);

    let mut w = [0u32; 64];
    for (i, chunk) in block.chunks_exact(4).enumerate().take(16) {
        w[i] = u32::from_be_bytes(chunk.try_into().expect("4 bytes"));
    }
    for i in 16..64 {
        w[i] = sig1(w[i - 2])
            .wrapping_add(w[i - 7])
            .wrapping_add(sig0(w[i - 15]))
            .wrapping_add(w[i - 16]);
    }

    let mut a = state[0];
    let mut b = state[1];
    let mut c = state[2];
    let mut d = state[3];
    let mut e = state[4];
    let mut f = state[5];
    let mut g = state[6];
    let mut h = state[7];

    for i in 0..64 {
        let t1 = h
            .wrapping_add(ep1(e))
            .wrapping_add(ch(e, f, g))
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let t2 = ep0(a).wrapping_add(maj(a, b, c));
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

#[inline]
fn rotr(x: u32, n: u32) -> u32 {
    x.rotate_right(n)
}

#[inline]
fn ch(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (!x & z)
}

#[inline]
fn maj(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (x & z) ^ (y & z)
}

#[inline]
fn ep0(x: u32) -> u32 {
    rotr(x, 2) ^ rotr(x, 13) ^ rotr(x, 22)
}

#[inline]
fn ep1(x: u32) -> u32 {
    rotr(x, 6) ^ rotr(x, 11) ^ rotr(x, 25)
}

#[inline]
fn sig0(x: u32) -> u32 {
    rotr(x, 7) ^ rotr(x, 18) ^ (x >> 3)
}

#[inline]
fn sig1(x: u32) -> u32 {
    rotr(x, 17) ^ rotr(x, 19) ^ (x >> 10)
}

// ---------------------------------------------------------------------------
// Content normalization + entry hashing
// ---------------------------------------------------------------------------

/// Normalize line endings to `\n` and trim trailing whitespace on each line.
pub fn normalize_text(text: &str) -> String {
    let unified = text.replace("\r\n", "\n").replace('\r', "\n");
    unified
        .lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

/// Normalize HTML bytes the same way as text (line endings + trailing trim).
pub fn normalize_html(html: &str) -> String {
    normalize_text(html)
}

/// Hash normalized plain text (UTF-8 bytes).
pub fn hash_text(text: &str) -> [u8; 32] {
    sha256(normalize_text(text).as_bytes())
}

/// Hash normalized HTML fragment bytes.
pub fn hash_html(html: &str) -> [u8; 32] {
    sha256(normalize_html(html).as_bytes())
}

/// Hash raw BGRA pixel bytes from the capture pipeline.
pub fn hash_image_pixels(pixels: &[u8]) -> [u8; 32] {
    sha256(pixels)
}

/// Compute the content hash for a clipboard entry before it is stored.
pub fn hash_clip_entry(
    kind: EntryKind,
    text: Option<&str>,
    html: Option<&str>,
    image_pixels: Option<&[u8]>,
) -> [u8; 32] {
    match kind {
        EntryKind::Text => hash_text(text.unwrap_or("")),
        EntryKind::RichText => {
            if let Some(html) = html.filter(|h| !h.is_empty()) {
                hash_html(html)
            } else {
                hash_text(text.unwrap_or(""))
            }
        }
        EntryKind::Image => hash_image_pixels(image_pixels.unwrap_or(&[])),
    }
}

/// Lowercase hex encoding of a 32-byte digest (for blob filenames and `ImageRef`).
pub fn hash_to_hex(hash: [u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in hash {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Parse a lowercase hex digest produced by [`hash_to_hex`].
pub fn hex_to_hash(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks_exact(2).enumerate() {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Deduplication
// ---------------------------------------------------------------------------

/// Returns `true` when the entry should be dropped (duplicate content).
pub fn is_duplicate_entry(
    history: &[ClipEntry],
    entry: &ClipEntry,
    deduplicate_global: bool,
) -> bool {
    if let Some(recent) = history.first() {
        if recent.kind == entry.kind && recent.hash == entry.hash {
            return true;
        }
    }

    if deduplicate_global {
        return history.iter().any(|e| e.hash == entry.hash);
    }

    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ClipEntry, ImageRef};

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn sha256_empty_string() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_abc() {
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_multi_block() {
        let msg = "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(
            hex(&sha256(msg.as_bytes())),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn sha256_streaming_matches_one_shot() {
        let data = b"The quick brown fox jumps over the lazy dog".repeat(20);
        let one_shot = sha256(&data);

        let mut hasher = Sha256::new();
        for chunk in data.chunks(13) {
            hasher.update(chunk);
        }
        assert_eq!(hasher.finalize(), one_shot);
    }

    #[test]
    fn normalize_line_endings() {
        assert_eq!(normalize_text("a\r\nb\rc"), "a\nb\nc");
        assert_eq!(hash_text("a\r\nb"), hash_text("a\nb"));
    }

    #[test]
    fn normalize_trims_trailing_whitespace() {
        assert_eq!(normalize_text("hello   \nworld\t"), "hello\nworld");
    }

    #[test]
    fn hash_hex_round_trip() {
        let digest = sha256(b"round-trip");
        let hex = hash_to_hex(digest);
        assert_eq!(hex_to_hash(&hex), Some(digest));
        assert_eq!(hex_to_hash("not-hex"), None);
    }

    #[test]
    fn hash_to_hex_lowercase() {
        let digest = sha256(b"test");
        assert_eq!(hash_to_hex(digest).len(), 64);
        assert!(hash_to_hex(digest).chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hash_to_hex(digest), hex(&digest));
    }

    #[test]
    fn hash_image_uses_raw_pixels() {
        let pixels = [0u8, 1, 2, 3, 255];
        assert_eq!(hash_image_pixels(&pixels), sha256(&pixels));
    }

    fn sample_entry(kind: EntryKind, hash_byte: u8) -> ClipEntry {
        let mut hash = [0u8; 32];
        hash[0] = hash_byte;
        ClipEntry {
            id: 0,
            created_at: 0,
            kind,
            text: None,
            html: None,
            image: None,
            image_pixels: None,
            source_app: None,
            is_pinned: false,
            hash,
        }
    }

    #[test]
    fn dedup_consecutive_same_kind() {
        let history = vec![sample_entry(EntryKind::Text, 1)];
        let dup = sample_entry(EntryKind::Text, 1);
        let different = sample_entry(EntryKind::Text, 2);

        assert!(is_duplicate_entry(&history, &dup, false));
        assert!(!is_duplicate_entry(&history, &different, false));
    }

    #[test]
    fn dedup_different_kind_not_consecutive() {
        let history = vec![sample_entry(EntryKind::Text, 1)];
        let image_same_hash = sample_entry(EntryKind::Image, 1);
        assert!(!is_duplicate_entry(&history, &image_same_hash, false));
    }

    #[test]
    fn dedup_global_anywhere() {
        let history = vec![
            sample_entry(EntryKind::Text, 1),
            sample_entry(EntryKind::Image, 2),
        ];
        let dup = sample_entry(EntryKind::RichText, 2);
        assert!(!is_duplicate_entry(&history, &dup, false));
        assert!(is_duplicate_entry(&history, &dup, true));
    }

    #[test]
    fn hash_clip_entry_rich_text_prefers_html() {
        let html_hash = hash_html("<b>x</b>");
        let entry_hash = hash_clip_entry(
            EntryKind::RichText,
            Some("different plain"),
            Some("<b>x</b>"),
            None,
        );
        assert_eq!(entry_hash, html_hash);
    }

    #[test]
    fn hash_clip_entry_image() {
        let pixels = vec![0u8; 16];
        let entry_hash = hash_clip_entry(EntryKind::Image, None, None, Some(&pixels));
        assert_eq!(entry_hash, hash_image_pixels(&pixels));
    }

    #[test]
    fn image_ref_hash_is_hex_digest() {
        let pixels = vec![1u8, 2, 3, 4];
        let digest = hash_clip_entry(EntryKind::Image, None, None, Some(&pixels));
        let image_ref = ImageRef {
            hash: hash_to_hex(digest),
            width: 1,
            height: 1,
        };
        assert_eq!(image_ref.hash, hash_to_hex(digest));
    }
}
