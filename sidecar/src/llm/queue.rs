use tokio::sync::Semaphore;
use tracing::debug;

/// Bounded inference queue to prevent memory exhaustion from concurrent LLM requests.
pub struct InferenceQueue {
    semaphore: Semaphore,
    max_depth: usize,
}

/// Result of attempting to acquire a queue slot.
pub enum QueueResult {
    /// Acquired a slot; the permit is held until dropped.
    Acquired(tokio::sync::SemaphorePermit<'static>),
    /// Queue is full; caller should fall back to template.
    Full,
}

impl InferenceQueue {
    /// Create a new inference queue with the given maximum concurrent requests.
    pub fn new(max_depth: usize) -> Self {
        Self {
            semaphore: Semaphore::new(max_depth),
            max_depth,
        }
    }

    /// Try to acquire a slot in the queue without blocking.
    /// Returns `QueueResult::Full` if the queue is at capacity.
    pub fn try_acquire(&'static self) -> QueueResult {
        match self.semaphore.try_acquire() {
            Ok(permit) => {
                debug!(
                    "LLM queue slot acquired ({} available)",
                    self.semaphore.available_permits()
                );
                QueueResult::Acquired(permit)
            }
            Err(_) => {
                debug!("LLM queue full ({} max)", self.max_depth);
                QueueResult::Full
            }
        }
    }

    /// Number of available slots.
    pub fn available(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Maximum queue depth.
    pub fn max_depth(&self) -> usize {
        self.max_depth
    }
}
