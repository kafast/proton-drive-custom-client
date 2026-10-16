use std::fmt::Write;

use sha1::{Digest, Sha1};

/// SHA-1 as lowercase hex, the way Proton stores it.
pub(crate) fn sha1_hex(content: &[u8]) -> String {
    Sha1::digest(content)
        .iter()
        .fold(String::new(), |mut hex, byte| {
            // Writing to a String cannot fail.
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_known_digest() {
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }
}
