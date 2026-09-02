//! Disk-backed flat exact vector index utilizing memory mapping and parallel CPU scanning.
//!
//! # Performance & Memory Characteristics
//!
//! - **Zero Rust heap allocation for vector storage**: The entire index stays on disk and is paged
//!   in on demand by the OS kernel via `memmap2`.
//! - **Exact recall**: 100% semantic recall without ANN approximation errors or graph construction overhead.
//! - **Data-parallel SIMD scan**: Scales linearly with CPU cores via `rayon`, scanning 400,000 documents in milliseconds.

use memmap2::Mmap;
use rayon::prelude::*;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::fs::File;
use std::path::{Path, PathBuf};

use super::format::{
    compute_checksum, VectorHeader, VectorRecord, HEADER_SIZE, RECORD_SIZE, VECTOR_DIMS,
};
use super::manifest::VectorManifest;
use super::traits::{VectorHit, VectorSearch};
use crate::search::filters::SearchFilter;

/// Helper for maintaining a bounded min-heap of top-k items.
#[derive(Debug, Clone, PartialEq)]
struct MinHeapItem {
    search_id: u64,
    score: f32,
}

impl Eq for MinHeapItem {}

impl PartialOrd for MinHeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MinHeapItem {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse order so BinaryHeap behaves as a Min-Heap by score
        self.score
            .partial_cmp(&other.score)
            .unwrap_or(Ordering::Equal)
            .reverse()
    }
}

/// A memory-mapped exact flat vector index.
pub struct FlatExactVectorIndex {
    path: PathBuf,
    mmap: Mmap,
    header: VectorHeader,
}

impl FlatExactVectorIndex {
    /// Open a `vectors.bin` file and memory-map it.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path_buf = path.as_ref().to_path_buf();
        let file = File::open(&path_buf)
            .map_err(|e| format!("Failed to open vector file {}: {e}", path_buf.display()))?;

        let mmap = unsafe {
            Mmap::map(&file)
                .map_err(|e| format!("Failed to mmap vector file {}: {e}", path_buf.display()))?
        };

        if mmap.len() < HEADER_SIZE {
            return Err("Vector file smaller than header size".to_string());
        }

        let header = VectorHeader::from_bytes(&mmap[0..HEADER_SIZE])?;
        let record_count = usize::try_from(header.record_count)
            .map_err(|_| "Vector record count does not fit this platform".to_string())?;
        let body_len = record_count
            .checked_mul(RECORD_SIZE)
            .ok_or_else(|| "Vector body length overflow".to_string())?;
        let expected_len = HEADER_SIZE
            .checked_add(body_len)
            .ok_or_else(|| "Vector file length overflow".to_string())?;
        if mmap.len() != expected_len {
            return Err(format!(
                "Vector file length mismatch: expected exactly {expected_len} bytes, found {}",
                mmap.len()
            ));
        }
        let checksum = compute_checksum(&mmap[HEADER_SIZE..expected_len]);
        if checksum != header.checksum {
            return Err(format!(
                "Vector body checksum mismatch: expected {}, found {checksum}",
                header.checksum
            ));
        }

        Ok(Self {
            path: path_buf,
            mmap,
            header,
        })
    }

    /// Open a vector file only when its sibling metadata matches the supported model contract.
    pub fn open_with_manifest(
        path: impl AsRef<Path>,
        manifest_path: impl AsRef<Path>,
    ) -> Result<Self, String> {
        let manifest = VectorManifest::load(manifest_path)?;
        let index = Self::open(path)?;
        manifest.validate_for_header(&index.header)?;
        Ok(index)
    }

    /// Helper to create a new `vectors.bin` file from a list of records.
    pub fn write_new(
        path: impl AsRef<Path>,
        generation: u32,
        model_revision: &str,
        records: &[VectorRecord],
    ) -> Result<Self, String> {
        use std::io::Write;
        let mut file =
            File::create(&path).map_err(|e| format!("Failed to create vector file: {e}"))?;

        let mut header = VectorHeader::new(records.len() as u64, generation, model_revision);

        let mut body_bytes = Vec::with_capacity(records.len() * RECORD_SIZE);
        for rec in records {
            body_bytes.extend_from_slice(&rec.to_bytes());
        }
        header.checksum = super::format::compute_checksum(&body_bytes);

        file.write_all(&header.to_bytes())
            .map_err(|e| format!("Failed to write header: {e}"))?;
        file.write_all(&body_bytes)
            .map_err(|e| format!("Failed to write records: {e}"))?;
        file.flush()
            .map_err(|e| format!("Failed to flush vector file: {e}"))?;

        Self::open(path)
    }

    /// Create an in-memory/tempfile vector index directly from records.
    pub fn from_records(records: &[VectorRecord]) -> Result<Self, String> {
        let temp_path = std::env::temp_dir().join(format!(
            "preploop_vec_{}_{}.bin",
            std::process::id(),
            records.len()
        ));
        Self::write_new(&temp_path, 1, "bundled", records)
    }

    /// Returns the underlying file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read record at index `i` (0-indexed).
    #[inline]
    pub fn get_record(&self, i: usize) -> Result<VectorRecord, String> {
        let offset = HEADER_SIZE + i * RECORD_SIZE;
        VectorRecord::from_bytes(&self.mmap[offset..offset + RECORD_SIZE])
    }

    /// Materialize record metadata/vectors for validation or generation rebuilds.
    pub fn records(&self) -> Result<Vec<VectorRecord>, String> {
        (0..self.count())
            .map(|index| self.get_record(index))
            .collect()
    }

    /// Stream only the database identity fields without materializing vector payloads.
    pub fn record_metadata(&self) -> impl Iterator<Item = (u64, u64)> + '_ {
        (0..self.count()).map(|index| {
            let offset = HEADER_SIZE + index * RECORD_SIZE;
            let record = &self.mmap[offset..offset + 16];
            let search_id = u64::from_le_bytes(record[0..8].try_into().unwrap());
            let fingerprint = u64::from_le_bytes(record[8..16].try_into().unwrap());
            (search_id, fingerprint)
        })
    }
}

impl VectorSearch for FlatExactVectorIndex {
    fn count(&self) -> usize {
        self.header.record_count as usize
    }

    fn generation(&self) -> u32 {
        self.header.generation
    }

    fn search(
        &self,
        query: &[f32],
        filters: &SearchFilter,
        limit: usize,
    ) -> Result<Vec<VectorHit>, String> {
        if query.len() != VECTOR_DIMS {
            return Err(format!(
                "Query dimension mismatch: expected {VECTOR_DIMS}, got {}",
                query.len()
            ));
        }
        if limit == 0 || self.header.record_count == 0 {
            return Ok(Vec::new());
        }

        let num_records = self.header.record_count as usize;
        let mmap_slice = &self.mmap[HEADER_SIZE..HEADER_SIZE + num_records * RECORD_SIZE];

        // For small indexes (< 4000 questions), process directly on single thread to avoid thread scheduling overhead.
        // For large corpora (up to 400k), use Rayon parallel chunked reduction.
        let chunk_size = 16_384.max(num_records / rayon::current_num_threads().max(1));

        let top_candidates: BinaryHeap<MinHeapItem> = mmap_slice
            .par_chunks(chunk_size * RECORD_SIZE)
            .map(|chunk_bytes| {
                let records_in_chunk = chunk_bytes.len() / RECORD_SIZE;
                let mut local_heap: BinaryHeap<MinHeapItem> = BinaryHeap::with_capacity(limit + 1);

                for i in 0..records_in_chunk {
                    let rec_bytes = &chunk_bytes[i * RECORD_SIZE..(i + 1) * RECORD_SIZE];

                    let flags = u32::from_le_bytes(rec_bytes[20..24].try_into().unwrap());
                    if (flags & super::format::FLAG_STALE) != 0 {
                        continue;
                    }

                    let search_id = u64::from_le_bytes(rec_bytes[0..8].try_into().unwrap());
                    if !filters.allows_search_id(search_id) {
                        continue;
                    }
                    let inverse_norm = f32::from_le_bytes(rec_bytes[16..20].try_into().unwrap());
                    let vec_slice = &rec_bytes[24..24 + VECTOR_DIMS];

                    // Unrolled dot-product accumulator for autovectorization
                    let mut dot = 0.0_f32;
                    for j in 0..VECTOR_DIMS {
                        dot += query[j] * (vec_slice[j] as i8 as f32);
                    }

                    let score = dot * inverse_norm;

                    if local_heap.len() < limit {
                        local_heap.push(MinHeapItem { search_id, score });
                    } else if let Some(min_item) = local_heap.peek() {
                        if score > min_item.score {
                            local_heap.pop();
                            local_heap.push(MinHeapItem { search_id, score });
                        }
                    }
                }
                local_heap
            })
            .reduce(
                || BinaryHeap::with_capacity(limit + 1),
                |mut h1, h2| {
                    for item in h2 {
                        if h1.len() < limit {
                            h1.push(item);
                        } else if let Some(min_item) = h1.peek() {
                            if item.score > min_item.score {
                                h1.pop();
                                h1.push(item);
                            }
                        }
                    }
                    h1
                },
            );

        // Convert the merged min-heap to sorted VectorHit vector (highest score first)
        let mut results = Vec::with_capacity(top_candidates.len());
        for item in top_candidates.into_sorted_vec() {
            results.push(VectorHit {
                search_id: item.search_id,
                score: item.score,
            });
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record() -> VectorRecord {
        let mut vector = vec![0.0f32; VECTOR_DIMS];
        vector[0] = 1.0;
        VectorRecord::from_embedding(1, 0x1234, 0, &vector).unwrap()
    }

    fn temp_vector_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("test_vec_{label}_{}.bin", uuid::Uuid::new_v4()))
    }

    #[test]
    fn test_flat_vector_search_end_to_end() {
        let temp_path = std::env::temp_dir().join(format!("test_vec_{}.bin", uuid::Uuid::new_v4()));

        let mut v1 = vec![0.0f32; VECTOR_DIMS];
        v1[0] = 1.0;

        let mut v2 = vec![0.0f32; VECTOR_DIMS];
        v2[1] = 1.0;

        let mut v3 = vec![0.0f32; VECTOR_DIMS];
        v3[0] = -1.0;

        let r1 = VectorRecord::from_embedding(101, 1, 0, &v1).unwrap();
        let r2 = VectorRecord::from_embedding(102, 2, 0, &v2).unwrap();
        let r3 = VectorRecord::from_embedding(103, 3, 0, &v3).unwrap();

        let index = FlatExactVectorIndex::write_new(
            &temp_path,
            1,
            "2ab6fa8ea2d674564defd37171ae19079b864b33",
            &[r1, r2, r3],
        )
        .unwrap();

        assert_eq!(index.count(), 3);
        assert_eq!(index.generation(), 1);
        assert_eq!(
            index.record_metadata().collect::<Vec<_>>(),
            vec![(101, 1), (102, 2), (103, 3)]
        );

        let mut q = vec![0.0f32; VECTOR_DIMS];
        q[0] = 1.0;

        let hits = index.search(&q, &SearchFilter::default(), 10).unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].search_id, 101);
        assert!(
            (hits[0].score - 1.0).abs() < 0.02,
            "expected cosine ~ 1.0, got {}",
            hits[0].score
        );

        assert_eq!(hits[1].search_id, 102);
        assert!(
            hits[1].score.abs() < 0.02,
            "expected cosine ~ 0.0, got {}",
            hits[1].score
        );

        assert_eq!(hits[2].search_id, 103);
        assert!(
            (hits[2].score - (-1.0)).abs() < 0.02,
            "expected cosine ~ -1.0, got {}",
            hits[2].score
        );

        let filtered = SearchFilter {
            allowed_search_ids: Some(std::sync::Arc::new([102_u64].into_iter().collect())),
            ..Default::default()
        };
        let filtered_hits = index.search(&q, &filtered, 10).unwrap();
        assert_eq!(filtered_hits.len(), 1);
        assert_eq!(filtered_hits[0].search_id, 102);

        let _ = std::fs::remove_file(&temp_path);
    }

    #[test]
    fn test_open_rejects_corrupted_vector_body() {
        let path = temp_vector_path("corrupt");
        let index = FlatExactVectorIndex::write_new(
            &path,
            1,
            super::super::manifest::GRANITE_MODEL_REVISION,
            &[sample_record()],
        )
        .unwrap();
        drop(index);

        let mut bytes = std::fs::read(&path).unwrap();
        bytes[HEADER_SIZE + 24] ^= 1;
        std::fs::write(&path, bytes).unwrap();

        let error = FlatExactVectorIndex::open(&path).err().unwrap();
        assert!(error.contains("Vector body checksum mismatch"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_open_rejects_trailing_bytes() {
        let path = temp_vector_path("trailing");
        let index = FlatExactVectorIndex::write_new(
            &path,
            1,
            super::super::manifest::GRANITE_MODEL_REVISION,
            &[sample_record()],
        )
        .unwrap();
        drop(index);

        let mut bytes = std::fs::read(&path).unwrap();
        bytes.push(0);
        std::fs::write(&path, bytes).unwrap();

        let error = FlatExactVectorIndex::open(&path).err().unwrap();
        assert!(error.contains("expected exactly"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_open_with_manifest_rejects_incompatible_metadata() {
        let path = temp_vector_path("manifest");
        let manifest_path = path.with_extension("json");
        let index = FlatExactVectorIndex::write_new(
            &path,
            1,
            super::super::manifest::GRANITE_MODEL_REVISION,
            &[sample_record()],
        )
        .unwrap();
        drop(index);

        let mut manifest = VectorManifest::new_granite_q8(1, 1);
        manifest.record_count = 2;
        manifest.save(&manifest_path).unwrap();

        let error = FlatExactVectorIndex::open_with_manifest(&path, &manifest_path)
            .err()
            .unwrap();
        assert!(error.contains("Manifest record count mismatch"));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&manifest_path);
    }

    #[test]
    fn bundled_generation_satisfies_integrity_contract() {
        let generation_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models/search-index/generation-001");
        let index = FlatExactVectorIndex::open_with_manifest(
            generation_dir.join("vectors.bin"),
            generation_dir.join("manifest.json"),
        )
        .unwrap();

        assert!(index.count() > 0);
    }
}
