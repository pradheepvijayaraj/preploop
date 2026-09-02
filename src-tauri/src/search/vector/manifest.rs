//! Search generation manifest (manifest.json) serialization and validation.

use serde::{Deserialize, Serialize};
use std::path::Path;

use super::format::{VectorHeader, FORMAT_VERSION, VECTOR_DIMS};

pub const GRANITE_MODEL: &str = "ibm-granite/granite-embedding-small-english-r2";
pub const GRANITE_MODEL_REVISION: &str = "2ab6fa8ea2d674564defd37171ae19079b864b33";
pub const GRANITE_POOLING: &str = "cls";
pub const GRANITE_NORMALIZATION: &str = "l2";
pub const INT8_QUANTIZATION: &str = "int8";
pub const DOCUMENT_FORMAT_VERSION: u16 = 1;
pub const TAXONOMY_VERSION: u16 = crate::taxonomy::TAXONOMY_VERSION;

/// Metadata manifest stored with each index generation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VectorManifest {
    pub format_version: u16,
    pub dimensions: u16,
    pub quantization: String,
    pub model: String,
    pub model_revision: String,
    pub pooling: String,
    pub normalization: String,
    pub document_format_version: u16,
    pub taxonomy_version: u16,
    pub content_version: u64,
    pub record_count: u64,
    pub created_at: i64,
}

impl VectorManifest {
    pub fn new_granite_q8(record_count: u64, content_version: u64) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        Self {
            format_version: FORMAT_VERSION,
            dimensions: VECTOR_DIMS as u16,
            quantization: INT8_QUANTIZATION.to_string(),
            model: GRANITE_MODEL.to_string(),
            model_revision: GRANITE_MODEL_REVISION.to_string(),
            pooling: GRANITE_POOLING.to_string(),
            normalization: GRANITE_NORMALIZATION.to_string(),
            document_format_version: DOCUMENT_FORMAT_VERSION,
            taxonomy_version: TAXONOMY_VERSION,
            content_version,
            record_count,
            created_at: now,
        }
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let json_str = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize manifest: {e}"))?;
        std::fs::write(path, json_str)
            .map_err(|e| format!("Failed to write manifest file: {e}"))?;
        Ok(())
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read manifest file: {e}"))?;
        let manifest =
            serde_json::from_str(&content).map_err(|e| format!("Failed to parse manifest: {e}"))?;
        Ok(manifest)
    }

    /// Validate metadata that must match the embedding engine and vector body.
    pub fn validate_for_header(&self, header: &VectorHeader) -> Result<(), String> {
        if self.format_version != FORMAT_VERSION {
            return Err(format!(
                "Manifest format version mismatch: expected {FORMAT_VERSION}, found {}",
                self.format_version
            ));
        }
        if self.dimensions as usize != VECTOR_DIMS {
            return Err(format!(
                "Manifest dimension mismatch: expected {VECTOR_DIMS}, found {}",
                self.dimensions
            ));
        }
        if self.quantization != INT8_QUANTIZATION {
            return Err(format!(
                "Manifest quantization mismatch: expected {INT8_QUANTIZATION}, found {}",
                self.quantization
            ));
        }
        if self.model != GRANITE_MODEL {
            return Err(format!(
                "Manifest model mismatch: expected {GRANITE_MODEL}, found {}",
                self.model
            ));
        }
        if self.model_revision != GRANITE_MODEL_REVISION {
            return Err(format!(
                "Manifest model revision mismatch: expected {GRANITE_MODEL_REVISION}, found {}",
                self.model_revision
            ));
        }
        if self.pooling != GRANITE_POOLING {
            return Err(format!(
                "Manifest pooling mismatch: expected {GRANITE_POOLING}, found {}",
                self.pooling
            ));
        }
        if self.normalization != GRANITE_NORMALIZATION {
            return Err(format!(
                "Manifest normalization mismatch: expected {GRANITE_NORMALIZATION}, found {}",
                self.normalization
            ));
        }
        if self.document_format_version != DOCUMENT_FORMAT_VERSION {
            return Err(format!(
                "Manifest document format mismatch: expected {DOCUMENT_FORMAT_VERSION}, found {}",
                self.document_format_version
            ));
        }
        if self.taxonomy_version != TAXONOMY_VERSION {
            return Err(format!(
                "Manifest taxonomy version mismatch: expected {TAXONOMY_VERSION}, found {}",
                self.taxonomy_version
            ));
        }
        if self.record_count != header.record_count {
            return Err(format!(
                "Manifest record count mismatch: expected {}, found {}",
                header.record_count, self.record_count
            ));
        }

        let header_revision = header.model_revision()?;
        if header_revision != self.model_revision {
            return Err(format!(
                "Vector model revision {header_revision} does not match manifest revision {}",
                self.model_revision
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_serde() {
        let manifest = VectorManifest::new_granite_q8(4107, 1);
        let serialized = serde_json::to_string(&manifest).unwrap();
        let parsed: VectorManifest = serde_json::from_str(&serialized).unwrap();
        assert_eq!(manifest, parsed);
        assert_eq!(parsed.dimensions, 384);
        assert_eq!(parsed.quantization, "int8");
    }

    #[test]
    fn test_manifest_rejects_incompatible_model_metadata() {
        let header = VectorHeader::new(1, 1, GRANITE_MODEL_REVISION);
        let mut manifest = VectorManifest::new_granite_q8(1, 1);
        manifest.pooling = "mean".to_string();

        let error = manifest.validate_for_header(&header).unwrap_err();
        assert!(error.contains("Manifest pooling mismatch"));
    }

    #[test]
    fn test_manifest_rejects_header_model_revision_mismatch() {
        let header = VectorHeader::new(1, 1, "different-model-revision");
        let manifest = VectorManifest::new_granite_q8(1, 1);

        let error = manifest.validate_for_header(&header).unwrap_err();
        assert!(error.contains("does not match manifest revision"));
    }
}
