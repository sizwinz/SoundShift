use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::time::sleep;

/// Throttles search requests to 3 concurrent workers with dispatch interval
/// and adaptive exponential backoff on HTTP 429 per D-08.
#[derive(Clone)]
pub struct SearchRateLimiter {
    semaphore: Arc<Semaphore>,
    dispatch_interval: Duration,
    last_dispatch: Arc<Mutex<Instant>>,
}

impl SearchRateLimiter {
    pub fn new(max_concurrency: usize, dispatch_interval_ms: u64) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrency)),
            dispatch_interval: Duration::from_millis(dispatch_interval_ms),
            last_dispatch: Arc::new(Mutex::new(Instant::now().checked_sub(Duration::from_secs(10)).unwrap_or_else(Instant::now))),
        }
    }

    /// Acquires a worker permit while ensuring the minimum dispatch interval has elapsed.
    pub async fn acquire(&self) -> Result<OwnedSemaphorePermit, String> {
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| format!("Failed to acquire rate limiter permit: {}", e))?;

        let mut last = self.last_dispatch.lock().await;
        let elapsed = last.elapsed();
        if elapsed < self.dispatch_interval {
            sleep(self.dispatch_interval - elapsed).await;
        }
        *last = Instant::now();

        Ok(permit)
    }

    /// Backs off exponentially when rate-limited (HTTP 429) per D-08.
    pub async fn backoff(&self, attempt: u32) {
        let factor = 2u64.pow(attempt.min(4));
        let wait_ms = (1000 * factor).min(16000);
        sleep(Duration::from_millis(wait_ms)).await;
    }
}

impl Default for SearchRateLimiter {
    fn default() -> Self {
        Self::new(3, 175)
    }
}
