//! Binary vector file format layout and record serialization.
//!
//! # Format specification
//!
//! Fixed-width header (128 bytes) followed by N contiguous fixed-width records (408 bytes each).
//!
//! ## Header layout (128 bytes, 64-byte aligned)
//! - `magic`: `[u8; 8]` = `b"PLOOPV2\0"`
//! - `format_version`: `u16` = 1
//! - `dimensions`: `u16` = 384
//! - `record_size`: `u32` = 408
//! - `record_count`: `u64`
//! - `quantization`: `u8` = 0 (int8)
//! - `_reserved`: `[u8; 5]`
//! - `model_rev`: `[u8; 40]` (SHA-1 / revision identifier)
//! - `generation`: `u32`
//! - `checksum`: `u32` (Adler-32 over all records)
//! - `_pad`: `[u8; 50]`
//!
//! ## Record layout (408 bytes)
//! - `search_id`: `u64` (8 bytes) — links to `search_documents.search_id`
//! - `fingerprint`: `u64` (8 bytes) — FNV-1a content hash
//! - `inverse_norm`: `f32` (4 bytes) — dequantization multiplier `1.0 / (multiplier * query_multiplier)`
//! - `flags`: `u32` (4 bytes) — bit 0: deleted/stale; bits 1-31: encoded section/stage/year
//! - `vector`: `[i8; 384]` (384 bytes) — int8 quantized unit vector

pub const HEADER_MAGIC: &[u8; 8] = b"PLOOPV2\0";
pub const HEADER_SIZE: usize = 128;
pub const RECORD_SIZE: usize = 408;
pub const VECTOR_DIMS: usize = 384;
pub const FORMAT_VERSION: u16 = 1;
pub const QUANTIZATION_INT8: u8 = 0;

pub const FLAG_STALE: u32 = 1 << 0;

/// Header of the `vectors.bin` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorHeader {
    pub format_version: u16,
    pub dimensions: u16,
    pub record_size: u32,
    pub record_count: u64,
    pub quantization: u8,
    pub model_rev: [u8; 40],
    pub generation: u32,
    pub checksum: u32,
}

impl VectorHeader {
    pub fn new(record_count: u64, generation: u32, model_revision: &str) -> Self {
        let mut model_rev = [0u8; 40];
        let bytes = model_revision.as_bytes();
        let len = bytes.len().min(40);
        model_rev[..len].copy_from_slice(&bytes[..len]);

        Self {
            format_version: FORMAT_VERSION,
            dimensions: VECTOR_DIMS as u16,
            record_size: RECORD_SIZE as u32,
            record_count,
            quantization: QUANTIZATION_INT8,
            model_rev,
            generation,
            checksum: 0,
        }
    }

    pub fn to_bytes(&self) -> [u8; HEADER_SIZE] {
        let mut buf = [0u8; HEADER_SIZE];
        buf[0..8].copy_from_slice(HEADER_MAGIC);
        buf[8..10].copy_from_slice(&self.format_version.to_le_bytes());
        buf[10..12].copy_from_slice(&self.dimensions.to_le_bytes());
        buf[12..16].copy_from_slice(&self.record_size.to_le_bytes());
        buf[16..24].copy_from_slice(&self.record_count.to_le_bytes());
        buf[24] = self.quantization;
        // 25..30 reserved (5 bytes)
        buf[30..70].copy_from_slice(&self.model_rev);
        buf[70..74].copy_from_slice(&self.generation.to_le_bytes());
        buf[74..78].copy_from_slice(&self.checksum.to_le_bytes());
        // 78..128 padding (50 bytes)
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Result<Self, String> {
        if buf.len() < HEADER_SIZE {
            return Err("Header buffer too small".to_string());
        }
        if &buf[0..8] != HEADER_MAGIC {
            return Err("Invalid vector index magic".to_string());
        }
        let format_version = u16::from_le_bytes(buf[8..10].try_into().unwrap());
        if format_version != FORMAT_VERSION {
            return Err(format!(
                "Unsupported vector format version {format_version}"
            ));
        }
        let dimensions = u16::from_le_bytes(buf[10..12].try_into().unwrap());
        if dimensions as usize != VECTOR_DIMS {
            return Err(format!(
                "Dimension mismatch: expected {VECTOR_DIMS}, got {dimensions}"
            ));
        }
        let record_size = u32::from_le_bytes(buf[12..16].try_into().unwrap());
        if record_size as usize != RECORD_SIZE {
            return Err(format!(
                "Record size mismatch: expected {RECORD_SIZE}, got {record_size}"
            ));
        }
        let record_count = u64::from_le_bytes(buf[16..24].try_into().unwrap());
        let quantization = buf[24];
        if quantization != QUANTIZATION_INT8 {
            return Err(format!(
                "Unsupported vector quantization {quantization}: expected int8 ({QUANTIZATION_INT8})"
            ));
        }
        let mut model_rev = [0u8; 40];
        model_rev.copy_from_slice(&buf[30..70]);
        let generation = u32::from_le_bytes(buf[70..74].try_into().unwrap());
        let checksum = u32::from_le_bytes(buf[74..78].try_into().unwrap());

        Ok(Self {
            format_version,
            dimensions,
            record_size,
            record_count,
            quantization,
            model_rev,
            generation,
            checksum,
        })
    }

    /// Decode the NUL-padded model revision stored in the fixed-width header.
    pub fn model_revision(&self) -> Result<&str, String> {
        let end = self
            .model_rev
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(self.model_rev.len());
        if self.model_rev[end..].iter().any(|byte| *byte != 0) {
            return Err("Vector model revision contains non-zero bytes after padding".to_string());
        }
        std::str::from_utf8(&self.model_rev[..end])
            .map_err(|_| "Vector model revision is not valid UTF-8".to_string())
    }
}

/// In-memory representation of a single fixed-width vector record.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorRecord {
    pub search_id: u64,
    pub fingerprint: u64,
    pub inverse_norm: f32,
    pub flags: u32,
    pub vector: [i8; VECTOR_DIMS],
}

impl VectorRecord {
    /// Quantize an L2-normalized float vector into an int8 record.
    pub fn from_embedding(
        search_id: u64,
        fingerprint: u64,
        flags: u32,
        embedding: &[f32],
    ) -> Result<Self, String> {
        if embedding.len() != VECTOR_DIMS {
            return Err(format!(
                "Expected {VECTOR_DIMS} dimensions, found {}",
                embedding.len()
            ));
        }

        let max_abs = embedding.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
        let multiplier = if max_abs > 0.0 { 127.0 / max_abs } else { 1.0 };
        let mut int8_vec = [0i8; VECTOR_DIMS];
        let mut sum_sq = 0.0_f32;

        for (i, &val) in embedding.iter().enumerate() {
            let q = (val * multiplier).round().clamp(-128.0, 127.0) as i8;
            int8_vec[i] = q;
            sum_sq += (q as f32) * (q as f32);
        }

        let inverse_norm = if sum_sq > 0.0 {
            1.0 / sum_sq.sqrt()
        } else {
            0.0
        };

        Ok(Self {
            search_id,
            fingerprint,
            inverse_norm,
            flags,
            vector: int8_vec,
        })
    }

    /// Construct a VectorRecord directly from precomputed quantized components.
    pub fn from_quantized(
        search_id: u64,
        fingerprint: u64,
        inverse_norm: f32,
        flags: u32,
        vector: [i8; VECTOR_DIMS],
    ) -> Self {
        Self {
            search_id,
            fingerprint,
            inverse_norm,
            flags,
            vector,
        }
    }

    pub fn to_bytes(&self) -> [u8; RECORD_SIZE] {
        let mut buf = [0u8; RECORD_SIZE];
        buf[0..8].copy_from_slice(&self.search_id.to_le_bytes());
        buf[8..16].copy_from_slice(&self.fingerprint.to_le_bytes());
        buf[16..20].copy_from_slice(&self.inverse_norm.to_le_bytes());
        buf[20..24].copy_from_slice(&self.flags.to_le_bytes());
        for (i, &b) in self.vector.iter().enumerate() {
            buf[24 + i] = b as u8;
        }
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Result<Self, String> {
        if buf.len() < RECORD_SIZE {
            return Err("Record buffer too small".to_string());
        }
        let search_id = u64::from_le_bytes(buf[0..8].try_into().unwrap());
        let fingerprint = u64::from_le_bytes(buf[8..16].try_into().unwrap());
        let inverse_norm = f32::from_le_bytes(buf[16..20].try_into().unwrap());
        let flags = u32::from_le_bytes(buf[20..24].try_into().unwrap());
        let mut vector = [0i8; VECTOR_DIMS];
        for i in 0..VECTOR_DIMS {
            vector[i] = buf[24 + i] as i8;
        }

        Ok(Self {
            search_id,
            fingerprint,
            inverse_norm,
            flags,
            vector,
        })
    }
}

/// Computes an Adler-32 checksum over the body bytes.
pub fn compute_checksum(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_roundtrip() {
        let header = VectorHeader::new(4107, 1, "2ab6fa8ea2d674564defd37171ae19079b864b33");
        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), HEADER_SIZE);

        let parsed = VectorHeader::from_bytes(&bytes).unwrap();
        assert_eq!(header, parsed);
        assert_eq!(
            parsed.model_revision().unwrap(),
            "2ab6fa8ea2d674564defd37171ae19079b864b33"
        );
    }

    #[test]
    fn test_header_rejects_unsupported_quantization() {
        let mut bytes = VectorHeader::new(1, 1, "bundled").to_bytes();
        bytes[24] = 1;

        let error = VectorHeader::from_bytes(&bytes).unwrap_err();
        assert!(error.contains("Unsupported vector quantization"));
    }

    #[test]
    fn test_record_quantization_and_roundtrip() {
        let mut raw_vec = vec![0.0f32; VECTOR_DIMS];
        raw_vec[0] = 0.5;
        raw_vec[1] = -0.5;
        raw_vec[2] = std::f32::consts::FRAC_1_SQRT_2;

        let record = VectorRecord::from_embedding(42, 0x123456789ABCDEF0, 0, &raw_vec).unwrap();
        let bytes = record.to_bytes();
        assert_eq!(bytes.len(), RECORD_SIZE);

        let parsed = VectorRecord::from_bytes(&bytes).unwrap();
        assert_eq!(record.search_id, parsed.search_id);
        assert_eq!(record.fingerprint, parsed.fingerprint);
        assert_eq!(record.flags, parsed.flags);
        assert_eq!(record.vector, parsed.vector);
        assert!((record.inverse_norm - parsed.inverse_norm).abs() < 1e-6);
    }
}
