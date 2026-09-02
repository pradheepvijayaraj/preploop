//! Content fingerprinting for change detection during incremental indexing.

/// 64-bit FNV-1a content fingerprint.
pub fn content_fingerprint(text: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fingerprint_deterministic_and_sensitive() {
        let text1 = "Sample question prompt";
        let text2 = "Sample question prompt ";
        let text3 = "Different question prompt";

        let fp1 = content_fingerprint(text1);
        let fp2 = content_fingerprint(text2);
        let fp3 = content_fingerprint(text3);

        assert_eq!(fp1, content_fingerprint(text1));
        assert_ne!(fp1, fp2);
        assert_ne!(fp1, fp3);
    }
}
