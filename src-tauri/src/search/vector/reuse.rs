//! Bind compatible cached embeddings to the current database by content.
//! SQLite IDs and corpus membership can change across paper revisions. The
//! embedding depends only on canonical question/options text, so matching
//! fingerprints can reuse vectors from both the bundle and the local cache.

use std::collections::HashMap;

use super::flat::FlatExactVectorIndex;
use super::format::VectorRecord;
use super::traits::{VectorHit, VectorSearch};
use crate::search::filters::SearchFilter;

struct BoundSource {
    index: FlatExactVectorIndex,
    // One vector can serve several identical questions in different papers.
    // IDs are bound before filtering/top-k selection, not after retrieval.
    record_ids: Vec<Vec<u64>>,
}

/// Keeps vector payloads memory-mapped; only the local ID mapping is allocated.
pub(crate) struct ReusedVectorIndex {
    sources: Vec<BoundSource>,
    count: usize,
    generation: u32,
}

impl ReusedVectorIndex {
    /// Sources must already pass the model/format/checksum validation. The
    /// first source wins when several contain the same content fingerprint.
    pub fn new(indexes: Vec<FlatExactVectorIndex>, documents: &HashMap<u64, u64>) -> Self {
        let generation = indexes
            .iter()
            .map(VectorSearch::generation)
            .max()
            .unwrap_or(0);
        let mut by_content = HashMap::new();
        for (source, index) in indexes.iter().enumerate() {
            for (record, fingerprint) in index.reusable_record_metadata() {
                by_content.entry(fingerprint).or_insert((source, record));
            }
        }
        let mut sources = indexes
            .into_iter()
            .map(|index| BoundSource {
                record_ids: vec![Vec::new(); index.count()],
                index,
            })
            .collect::<Vec<_>>();
        let mut count = 0;
        for (&id, fingerprint) in documents {
            if let Some(&(source, record)) = by_content.get(fingerprint) {
                sources[source].record_ids[record].push(id);
                count += 1;
            }
        }
        sources.retain(|source| source.record_ids.iter().any(|ids| !ids.is_empty()));
        Self {
            sources,
            count,
            generation,
        }
    }

    /// Materialize only matching records, with current IDs, for persistence.
    pub fn records(&self) -> Result<Vec<VectorRecord>, String> {
        let mut records = Vec::with_capacity(self.count);
        for source in &self.sources {
            for (offset, ids) in source.record_ids.iter().enumerate() {
                if ids.is_empty() {
                    continue;
                }
                let record = source.index.get_record(offset)?;
                for &id in ids {
                    records.push(VectorRecord {
                        search_id: id,
                        ..record.clone()
                    });
                }
            }
        }
        records.sort_unstable_by_key(|record| record.search_id);
        Ok(records)
    }
}

impl VectorSearch for ReusedVectorIndex {
    fn count(&self) -> usize {
        self.count
    }

    fn generation(&self) -> u32 {
        self.generation
    }

    fn search(
        &self,
        query: &[f32],
        filters: &SearchFilter,
        limit: usize,
    ) -> Result<Vec<VectorHit>, String> {
        let mut hits = Vec::new();
        for source in &self.sources {
            hits.extend(
                source
                    .index
                    .search_with_ids(query, filters, limit, &source.record_ids)?,
            );
        }
        // Each local ID is assigned to exactly one source. Taking top-k from
        // each disjoint source retains the exact global top-k and tie order.
        hits.sort_unstable_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.search_id.cmp(&b.search_id))
        });
        hits.truncate(limit);
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::vector::format::{FLAG_STALE, VECTOR_DIMS};
    use crate::search::vector::manifest::GRANITE_MODEL_REVISION;
    use std::path::PathBuf;
    use std::sync::Arc;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("preploop_reuse_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn index(&self, name: &str, records: &[VectorRecord]) -> FlatExactVectorIndex {
            FlatExactVectorIndex::write_new(self.0.join(name), 1, GRANITE_MODEL_REVISION, records)
                .unwrap()
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn vector(axis: usize) -> Vec<f32> {
        let mut query = vec![0.0; VECTOR_DIMS];
        query[axis] = 1.0;
        query
    }

    fn record(id: u64, fingerprint: u64, axis: usize) -> VectorRecord {
        VectorRecord::from_embedding(id, fingerprint, 0, &vector(axis)).unwrap()
    }

    #[test]
    fn filters_and_limits_use_local_ids_including_identical_questions() {
        let dir = TestDirectory::new();
        let index = ReusedVectorIndex::new(
            vec![dir.index("bundle", &[record(1, 11, 0)])],
            &HashMap::from([(901, 11), (902, 11), (903, 11)]),
        );
        let hits = index
            .search(&vector(0), &SearchFilter::default(), 3)
            .unwrap();
        assert_eq!(
            hits.iter().map(|hit| hit.search_id).collect::<Vec<_>>(),
            [901, 902, 903]
        );
        let filters = SearchFilter {
            allowed_search_ids: Some(Arc::new([903].into_iter().collect())),
            ..Default::default()
        };
        let hits = index.search(&vector(0), &filters, 1).unwrap();
        assert_eq!(hits[0].search_id, 903);
        assert!(index.search(&vector(0), &filters, 0).unwrap().is_empty());
    }

    #[test]
    fn reuse_joins_sources_by_content_excludes_stale_content_and_deduplicates_ids() {
        let dir = TestDirectory::new();
        let mut stale = record(3, 33, 0);
        stale.flags = FLAG_STALE;
        let index = ReusedVectorIndex::new(
            vec![
                dir.index("local", &[record(1, 11, 0), stale]),
                dir.index("bundle", &[record(1, 22, 1), record(2, 11, 0)]),
            ],
            &HashMap::from([(101, 11), (102, 22), (103, 33), (1, 44)]),
        );
        assert_eq!(index.count(), 2);
        let hits = index
            .search(&vector(1), &SearchFilter::default(), 10)
            .unwrap();
        assert_eq!(
            hits.iter().map(|hit| hit.search_id).collect::<Vec<_>>(),
            [102, 101]
        );
        let records = index.records().unwrap();
        assert_eq!(records, [record(101, 11, 0), record(102, 22, 1)]);
    }

    #[test]
    fn remapped_parallel_scan_matches_a_materialized_index_across_chunk_boundaries() {
        let dir = TestDirectory::new();
        let records = (1..=16_500).map(|id| record(id, id, 0)).collect::<Vec<_>>();
        let mut documents = (1..=16_500)
            .map(|id| (50_000 - id, id))
            .collect::<HashMap<_, _>>();
        // Aliases in the last chunk must participate in the same top-k as
        // records from the first chunk, with ties broken by their local IDs.
        documents.insert(7, 16_500);
        documents.insert(8, 16_500);
        let index = ReusedVectorIndex::new(vec![dir.index("bundle", &records)], &documents);
        let materialized = dir.index("materialized", &index.records().unwrap());
        for allowed in [
            None,
            Some(Arc::new([8, 33_501, 49_999].into_iter().collect())),
        ] {
            let filters = SearchFilter {
                allowed_search_ids: allowed,
                ..Default::default()
            };
            for limit in [1, 3, 20] {
                assert_eq!(
                    index.search(&vector(0), &filters, limit).unwrap(),
                    materialized.search(&vector(0), &filters, limit).unwrap(),
                );
            }
        }
    }
}
