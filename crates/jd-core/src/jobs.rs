//! Running jobs: progress and cancellation shared by GUI, CLI and MCP (`06-jobs.md`, slice 1).
//!
//! ponytail: synchronous ops report progress through a shared table and poll a cancel flag; the
//! async scheduler (queues, budgets, timeouts, broadcast progress) comes when ops run detached.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use jd_graph::JobCtl;
use serde::{Deserialize, Serialize};

struct Entry {
    op: String,
    started: Instant,
    progress: Arc<Mutex<(usize, usize, String)>>,
    cancel: Arc<AtomicBool>,
}

/// A running job as reported by `jobs.list`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct JobInfo {
    /// Job id (process-local, increasing).
    pub id: u64,
    /// Operation running.
    pub op: String,
    /// Milliseconds since the job started.
    pub elapsed_ms: u64,
    /// Steps finished (graph nodes).
    pub done: usize,
    /// Total steps.
    pub total: usize,
    /// What is running now (node kind).
    pub current: String,
    /// Cancel requested.
    pub cancelling: bool,
}

/// Process-wide job table.
#[derive(Default)]
pub struct Jobs {
    next: AtomicU64,
    running: Mutex<BTreeMap<u64, Entry>>,
}

/// Removes the job from the table when the op returns (success, error or panic unwinding).
pub struct JobGuard<'a> {
    jobs: &'a Jobs,
    /// Job id, for error details.
    pub id: u64,
    /// Flag + progress sink to hand to the evaluator.
    pub ctl: JobCtl,
}

impl Drop for JobGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut r) = self.jobs.running.lock() {
            r.remove(&self.id);
        }
    }
}

impl Jobs {
    /// Register a job for `op`; the guard carries the control handle.
    pub fn start(&self, op: &str) -> JobGuard<'_> {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let progress = Arc::new(Mutex::new((0usize, 0usize, String::new())));
        let cancel = Arc::new(AtomicBool::new(false));
        if let Ok(mut r) = self.running.lock() {
            r.insert(
                id,
                Entry {
                    op: op.to_owned(),
                    started: Instant::now(),
                    progress: progress.clone(),
                    cancel: cancel.clone(),
                },
            );
        }
        let sink = progress.clone();
        let ctl = JobCtl {
            cancel,
            progress: Arc::new(move |done, total, current| {
                if let Ok(mut p) = sink.lock() {
                    *p = (done, total, current.to_owned());
                }
            }),
        };
        JobGuard {
            jobs: self,
            id,
            ctl,
        }
    }

    /// Snapshot of running jobs.
    pub fn list(&self) -> Vec<JobInfo> {
        let Ok(r) = self.running.lock() else {
            return Vec::new();
        };
        r.iter()
            .map(|(id, e)| {
                let (done, total, current) =
                    e.progress.lock().map(|p| p.clone()).unwrap_or_default();
                JobInfo {
                    id: *id,
                    op: e.op.clone(),
                    elapsed_ms: e.started.elapsed().as_millis() as u64,
                    done,
                    total,
                    current,
                    cancelling: e.cancel.load(Ordering::Relaxed),
                }
            })
            .collect()
    }

    /// Request cancellation; false when no such job is running.
    pub fn cancel(&self, id: u64) -> bool {
        self.running
            .lock()
            .ok()
            .and_then(|r| r.get(&id).map(|e| e.cancel.store(true, Ordering::Relaxed)))
            .is_some()
    }
}
