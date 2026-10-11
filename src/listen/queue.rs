use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};

/// Bytes of output that may wait for stdout before requests get 503.
pub const OUTPUT_BUDGET: usize = 8 * 1024 * 1024;

/// Output waiting for stdout, bounded in bytes, so a slow reader never
/// blocks an HTTP response. A dedicated thread writes entries in order.
pub struct Queue {
    tx: mpsc::UnboundedSender<(String, OwnedSemaphorePermit)>,
    budget: Arc<Semaphore>,
    capacity: usize,
}

impl Queue {
    pub fn new(capacity: usize, mut sink: impl FnMut(&str) + Send + 'static) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<(String, OwnedSemaphorePermit)>();
        std::thread::Builder::new()
            .name("teams-listen-output".into())
            .spawn(move || {
                // The permit is held until the write returns.
                while let Some((text, _permit)) = rx.blocking_recv() {
                    sink(&text);
                }
            })
            .expect("spawn output thread");
        Self {
            tx,
            budget: Arc::new(Semaphore::new(capacity)),
            capacity,
        }
    }

    /// Enqueue `text` whole, or return false when it does not fit.
    pub fn try_push(&self, text: String) -> bool {
        let Ok(n) = u32::try_from(text.len().max(1)) else {
            return false;
        };
        match self.budget.clone().try_acquire_many_owned(n) {
            Ok(permit) => self.tx.send((text, permit)).is_ok(),
            Err(_) => false,
        }
    }

    /// Wait up to `limit` for queued output to be written.
    pub async fn flush(&self, limit: Duration) {
        let _ = tokio::time::timeout(limit, async {
            while self.budget.available_permits() < self.capacity {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc as std_mpsc;

    #[tokio::test]
    async fn full_queue_refuses_until_the_writer_catches_up() {
        let (release_tx, release_rx) = std_mpsc::channel::<()>();
        let (out_tx, out_rx) = std_mpsc::channel::<String>();
        let queue = Queue::new(10, move |text| {
            release_rx.recv().unwrap();
            out_tx.send(text.to_string()).unwrap();
        });

        // The writer takes the first entry and blocks in the sink holding its
        // bytes; the second fills the budget.
        assert!(queue.try_push("aaaaa".into()));
        assert!(queue.try_push("bbbbb".into()));
        assert!(!queue.try_push("c".into()));

        release_tx.send(()).unwrap();
        assert_eq!(out_rx.recv().unwrap(), "aaaaa");
        // Space frees once the write returns.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !queue.try_push("c".into()) {
            assert!(std::time::Instant::now() < deadline, "space never freed");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        release_tx.send(()).unwrap();
        release_tx.send(()).unwrap();
        assert_eq!(out_rx.recv().unwrap(), "bbbbb");
        assert_eq!(out_rx.recv().unwrap(), "c");
    }

    #[test]
    fn entry_larger_than_the_budget_is_refused() {
        let queue = Queue::new(4, |_| {});
        assert!(!queue.try_push("12345".into()));
        assert!(queue.try_push("1234".into()));
    }
}
