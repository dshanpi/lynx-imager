use log::error;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::efex::error::EfexError;

static FEL_TIMEOUT_SECS: AtomicU64 = AtomicU64::new(1);
static FES_TIMEOUT_SECS: AtomicU64 = AtomicU64::new(1);

pub const DEFAULT_TIMEOUT_DURATION: Duration = Duration::from_secs(1);

pub fn set_fel_timeout(timeout_secs: u64) {
    FEL_TIMEOUT_SECS.store(timeout_secs, Ordering::SeqCst);
}

pub fn set_fes_timeout(timeout_secs: u64) {
    FES_TIMEOUT_SECS.store(timeout_secs, Ordering::SeqCst);
}

pub fn get_fel_timeout() -> Duration {
    Duration::from_secs(FEL_TIMEOUT_SECS.load(Ordering::SeqCst))
}

pub fn get_fes_timeout() -> Duration {
    Duration::from_secs(FES_TIMEOUT_SECS.load(Ordering::SeqCst))
}

pub async fn run_blocking_with_timeout<T, F>(
    timeout: Duration,
    timeout_message: &str,
    task: F,
) -> Result<T, EfexError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, EfexError> + Send + 'static,
{
    tokio::time::timeout(timeout, tokio::task::spawn_blocking(task))
        .await
        .map_err(|_| {
            error!("{}", timeout_message);
            EfexError::timeout(timeout_message)
        })?
        .map_err(|e| {
            error!("Task error: {}", e);
            EfexError {
                code: -1,
                name: "TaskError".to_string(),
                message: e.to_string(),
            }
        })?
}
