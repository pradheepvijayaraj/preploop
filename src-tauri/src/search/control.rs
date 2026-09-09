//! Cooperative cancellation for superseded search requests.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct SearchCancellation(Arc<AtomicBool>);

impl SearchCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn check(&self) -> Result<(), String> {
        if self.is_cancelled() {
            Err("Search cancelled".into())
        } else {
            Ok(())
        }
    }

    /// A non-allocating predicate for cancellation checkpoints and model loading.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Default)]
pub struct SearchRequests(Mutex<HashMap<String, (u64, SearchCancellation)>>);

impl SearchRequests {
    /// Register before spawning work so older jobs cannot replace newer input.
    pub fn begin(&self, client: &str, revision: u64) -> Result<SearchCancellation, String> {
        let mut requests = self.0.lock().map_err(|error| error.to_string())?;
        if let Some((latest, cancellation)) = requests.get(client) {
            if revision < *latest {
                return Err("Search cancelled".into());
            }
            cancellation.cancel();
        }
        let cancellation = SearchCancellation::default();
        requests.insert(client.into(), (revision, cancellation.clone()));
        Ok(cancellation)
    }

    /// Keep a revision tombstone to reject a delayed, already-superseded job.
    pub fn cancel_before(&self, client: &str, revision: u64) -> Result<(), String> {
        let mut requests = self.0.lock().map_err(|error| error.to_string())?;
        if let Some((latest, cancellation)) = requests.get(client) {
            if revision <= *latest {
                return Ok(());
            }
            cancellation.cancel();
        }
        let cancellation = SearchCancellation::default();
        cancellation.cancel();
        requests.insert(client.into(), (revision, cancellation));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_input_cancels_old_jobs_and_rejects_delayed_starts() {
        let requests = SearchRequests::default();
        let old = requests.begin("dialog", 1).unwrap();
        requests.cancel_before("dialog", 2).unwrap();
        assert!(old.check().is_err());
        assert!(requests.begin("dialog", 1).is_err());
        // The debounced job for this revision is still allowed to start.
        let current = requests.begin("dialog", 2).unwrap();
        requests.cancel_before("dialog", 2).unwrap();
        requests.cancel_before("dialog", 1).unwrap();
        assert!(current.check().is_ok());
        let other_window = requests.begin("other-dialog", 1).unwrap();
        requests.cancel_before("dialog", 3).unwrap();
        assert!(current.check().is_err());
        assert!(other_window.check().is_ok());
    }
}
