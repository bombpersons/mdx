//! Cooperative execution control. Budgets count attempted steps, including
//! interrupts and exceptions, and cancellation is independent of progress.
use crate::envelope::Failure;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct Context {
    /// Base directory captured for this call; never changed process-wide.
    pub root: PathBuf,
    pub cancellation: CancellationToken,
    pub store: PathBuf,
    pub progress: Option<Arc<dyn Fn(u64, u64) + Send + Sync>>,
    pub deadline: Option<Instant>,
}

impl Default for Context {
    fn default() -> Self {
        Self {
            root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            cancellation: CancellationToken::new(),
            store: PathBuf::from(".mdx/artifacts"),
            progress: None,
            deadline: None,
        }
    }
}

impl Context {
    pub fn bounded(&self, timeout_ms: Option<u64>) -> Result<Self, Failure> {
        let mut context = self.clone();
        if let Some(ms) = timeout_ms {
            if ms > 3_600_000 {
                return Err(Failure {
                    code: "limit_exceeded".into(),
                    message: "timeout_ms must be <= 3600000".into(),
                    remediation: None,
                });
            }
            let deadline = Instant::now() + Duration::from_millis(ms);
            context.deadline = Some(context.deadline.map_or(deadline, |old| old.min(deadline)));
        }
        Ok(context)
    }

    pub fn stop_reason(&self) -> Option<&'static str> {
        if self.cancellation.is_cancelled() {
            Some("cancelled")
        } else if self.deadline.is_some_and(|d| Instant::now() >= d) {
            Some("timeout")
        } else {
            None
        }
    }

    pub fn check(&self) -> Result<(), Failure> {
        match self.stop_reason() {
            Some(reason) => Err(Failure {
                code: reason.into(),
                message: format!("operation {reason}"),
                remediation: None,
            }),
            None => Ok(()),
        }
    }

    pub fn report(&self, completed: u64, total: u64) {
        if let Some(progress) = &self.progress {
            progress(completed, total);
        }
    }
}
