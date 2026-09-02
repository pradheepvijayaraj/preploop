//! Generation-based index rebuilding and atomic active generation swapping.

use crate::search::vector::flat::FlatExactVectorIndex;
use crate::search::vector::format::VectorRecord;
use crate::search::vector::manifest::{VectorManifest, GRANITE_MODEL_REVISION};
use std::path::{Path, PathBuf};

/// Fully written generation that is not visible to readers until activated.
#[derive(Debug)]
pub struct StagedGeneration {
    name: String,
    tmp_dir: PathBuf,
    final_dir: PathBuf,
}

/// Coordinates atomic generation-based index building and switching.
pub struct GenerationManager;

impl GenerationManager {
    /// Reads the currently active generation name from `<base_dir>/active`.
    pub fn get_active_generation(base_dir: impl AsRef<Path>) -> Option<String> {
        let active_file = base_dir.as_ref().join("active");
        std::fs::read_to_string(active_file)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Loads the currently active vector index, if one exists.
    pub fn load_active_index(
        base_dir: impl AsRef<Path>,
    ) -> Result<Option<FlatExactVectorIndex>, String> {
        let base = base_dir.as_ref();
        if let Some(active_gen) = Self::get_active_generation(base) {
            let gen_dir = base.join(active_gen);
            let vec_file = gen_dir.join("vectors.bin");
            if vec_file.exists() {
                let index = FlatExactVectorIndex::open_with_manifest(
                    vec_file,
                    gen_dir.join("manifest.json"),
                )?;
                return Ok(Some(index));
            }
        }
        Ok(None)
    }

    /// Clear only the pointer to an unusable generated index. Generation
    /// directories are disposable caches and can be reused or replaced by a
    /// later rebuild; historical question data is never stored here.
    pub fn deactivate_active_generation(base_dir: impl AsRef<Path>) -> Result<(), String> {
        let active_file = base_dir.as_ref().join("active");
        match std::fs::remove_file(active_file) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }

    /// Atomically builds a new generation and switches the active pointer.
    pub fn build_and_swap_generation(
        base_dir: impl AsRef<Path>,
        generation: u32,
        model_revision: &str,
        content_version: u64,
        records: &[VectorRecord],
    ) -> Result<FlatExactVectorIndex, String> {
        let staged = Self::stage_generation(
            base_dir,
            generation,
            model_revision,
            content_version,
            records,
        )?;
        Self::activate_staged_generation(staged)
    }

    /// Write a generation completely without changing the active pointer.
    /// This allows CPU and filesystem work to happen without holding SQLite's
    /// interactive-command mutex.
    pub fn stage_generation(
        base_dir: impl AsRef<Path>,
        generation: u32,
        model_revision: &str,
        content_version: u64,
        records: &[VectorRecord],
    ) -> Result<StagedGeneration, String> {
        if model_revision != GRANITE_MODEL_REVISION {
            return Err(format!(
                "Unsupported embedding model revision {model_revision}: expected {GRANITE_MODEL_REVISION}"
            ));
        }
        let base = base_dir.as_ref();
        std::fs::create_dir_all(base).map_err(|e| e.to_string())?;

        let gen_name = format!("generation-{:03}", generation);
        let tmp_dir = base.join(format!("{}.tmp", gen_name));
        let final_dir = base.join(&gen_name);

        if tmp_dir.exists() {
            let _ = std::fs::remove_dir_all(&tmp_dir);
        }
        std::fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;

        let write_result = (|| {
            let vec_path = tmp_dir.join("vectors.bin");
            FlatExactVectorIndex::write_new(&vec_path, generation, model_revision, records)?;

            let manifest_path = tmp_dir.join("manifest.json");
            let manifest = VectorManifest::new_granite_q8(records.len() as u64, content_version);
            manifest.save(manifest_path)
        })();
        if let Err(error) = write_result {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Err(error);
        }

        Ok(StagedGeneration {
            name: gen_name,
            tmp_dir,
            final_dir,
        })
    }

    /// Atomically publish a previously staged generation.
    pub fn activate_staged_generation(
        staged: StagedGeneration,
    ) -> Result<FlatExactVectorIndex, String> {
        let base = staged
            .final_dir
            .parent()
            .ok_or_else(|| "Staged generation has no parent directory".to_string())?;
        if staged.final_dir.exists() {
            std::fs::remove_dir_all(&staged.final_dir).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&staged.tmp_dir, &staged.final_dir).map_err(|e| e.to_string())?;

        let active_file = base.join("active");
        let active_tmp = base.join("active.tmp");
        std::fs::write(&active_tmp, &staged.name).map_err(|e| e.to_string())?;
        std::fs::rename(&active_tmp, &active_file).map_err(|e| e.to_string())?;

        let new_vec_path = staged.final_dir.join("vectors.bin");
        FlatExactVectorIndex::open_with_manifest(
            new_vec_path,
            staged.final_dir.join("manifest.json"),
        )
    }

    /// Remove an unpublished generation after its database snapshot became
    /// stale. This only deletes disposable index output.
    pub fn discard_staged_generation(staged: StagedGeneration) {
        let _ = std::fs::remove_dir_all(staged.tmp_dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::vector::format::VECTOR_DIMS;
    use crate::search::vector::traits::VectorSearch;

    #[test]
    fn test_generation_build_and_swap_atomic() {
        let temp_dir = std::env::temp_dir().join(format!("test_gen_{}", uuid::Uuid::new_v4()));
        let mut v = vec![0.0f32; VECTOR_DIMS];
        v[0] = 1.0;
        let r = VectorRecord::from_embedding(1, 0x1234, 0, &v).unwrap();

        let index1 = GenerationManager::build_and_swap_generation(
            &temp_dir,
            1,
            "2ab6fa8ea2d674564defd37171ae19079b864b33",
            1,
            std::slice::from_ref(&r),
        )
        .unwrap();

        assert_eq!(index1.generation(), 1);
        assert_eq!(
            GenerationManager::get_active_generation(&temp_dir),
            Some("generation-001".to_string())
        );

        // Swap to generation 2
        let index2 = GenerationManager::build_and_swap_generation(
            &temp_dir,
            2,
            "2ab6fa8ea2d674564defd37171ae19079b864b33",
            2,
            &[r],
        )
        .unwrap();

        assert_eq!(index2.generation(), 2);
        assert_eq!(
            GenerationManager::get_active_generation(&temp_dir),
            Some("generation-002".to_string())
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
