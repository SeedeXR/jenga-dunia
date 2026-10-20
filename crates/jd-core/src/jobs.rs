//! Running jobs: progress and cancellation shared by GUI, CLI and MCP (`06-jobs.md`, slice 1).
//!
//! Detached execution (`graph.evaluate_async`): the op spawns a thread holding an owned guard,
//! the caller gets `status: accepted` + `job_id`, polls `jobs.list` for progress and collects the
//! envelope with `jobs.result {id}` (kept for the last 64 finished jobs).
//!
//! ponytail: one thread per detached job, no queue/budget/timeout — the scheduler (06-jobs.md)
//! comes when agents submit more work than cores.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use jd_graph::JobCtl;
use serde::{Deserialize, Serialize};

struct Entry {
    op: String,
    started: Instant,
    progress: Arc<Mutex<(usize, usize, String)>>,
    cancel: Arc<AtomicBool>,
    /// Detached jobs wait here for a worker slot; synchronous ops are always running.
    running_now: Arc<AtomicBool>,
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
    /// `running`, or `queued` for a detached job waiting for a worker slot.
    pub state: String,
}

/// Process-wide job table.
pub struct Jobs {
    next: AtomicU64,
    running: Mutex<BTreeMap<u64, Entry>>,
    /// Envelopes of detached jobs that finished, until collected (bounded).
    finished: Mutex<BTreeMap<u64, serde_json::Value>>,
    /// Worker slots for detached jobs: (in use, max) + a condvar to wait on.
    slots: (Mutex<(usize, usize)>, Condvar),
    /// FIFO tickets: jobs start in submission order (next to serve, last issued, cancelled while queued).
    tickets: Mutex<(u64, u64, std::collections::BTreeSet<u64>)>,
}

impl Default for Jobs {
    fn default() -> Self {
        // Half the cores for background evaluations, so the interactive path stays responsive.
        let max = std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1));
        Self {
            next: AtomicU64::new(0),
            running: Mutex::new(BTreeMap::new()),
            finished: Mutex::new(BTreeMap::new()),
            slots: (Mutex::new((0, max)), Condvar::new()),
            tickets: Mutex::new((1, 0, std::collections::BTreeSet::new())),
        }
    }
}

/// Owned variant of [`JobGuard`] for detached jobs (lives on the worker thread).
pub struct OwnedJobGuard {
    jobs: Arc<Jobs>,
    /// Job id.
    pub id: u64,
    /// Flag + progress sink to hand to the evaluator.
    pub ctl: JobCtl,
}

impl Drop for OwnedJobGuard {
    fn drop(&mut self) {
        if let Ok(mut r) = self.jobs.running.lock() {
            r.remove(&self.id);
        }
    }
}

/// What `jobs.result` reports.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct JobResult {
    /// Job id asked for.
    pub id: u64,
    /// `running`, `done` (envelope attached, now collected) or `unknown` (never existed or already collected).
    pub state: String,
    /// Progress while running.
    pub info: Option<JobInfo>,
    /// The finished op's envelope fields: `status`, `result` / `error`, `warnings`.
    pub response: Option<serde_json::Value>,
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
                    running_now: Arc::new(AtomicBool::new(true)),
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

    /// Register a detached job; the guard is owned so a worker thread can carry it.
    pub fn start_owned(self: &Arc<Self>, op: &str) -> OwnedJobGuard {
        let g = self.start(op);
        let (id, ctl) = (g.id, g.ctl.clone());
        std::mem::forget(g); // the owned guard below removes the entry instead
        OwnedJobGuard {
            jobs: Arc::clone(self),
            id,
            ctl,
        }
    }

    /// Cap on concurrently running detached jobs (default: half the cores, at least 1).
    pub fn set_max_detached(&self, max: usize) {
        if let Ok(mut s) = self.slots.0.lock() {
            s.1 = max.max(1);
            self.slots.1.notify_all();
        }
    }

    /// Take a queue ticket at submission time (before the worker thread exists) so detached jobs
    /// start in the order they were requested.
    pub fn ticket(&self) -> u64 {
        let Ok(mut t) = self.tickets.lock() else {
            return 0;
        };
        t.1 += 1;
        t.1
    }

    /// Is `ticket` the next one to serve? Skips tickets that were cancelled while queued.
    fn at_head(t: &mut (u64, u64, std::collections::BTreeSet<u64>), ticket: u64) -> bool {
        while t.2.remove(&t.0) {
            t.0 += 1;
        }
        t.0 == ticket
    }

    /// Mark a detached job as waiting for a slot, then block until it is at the head of the queue
    /// and a slot is free. Returns false when the job was cancelled while queued.
    pub fn acquire_slot(&self, id: u64, ticket: u64, cancel: &AtomicBool) -> bool {
        let running_now = self
            .running
            .lock()
            .ok()
            .and_then(|r| r.get(&id).map(|e| e.running_now.clone()));
        if let Some(flag) = &running_now {
            flag.store(false, Ordering::Relaxed);
        }
        let give_up = |this: &Self| {
            if let Ok(mut t) = this.tickets.lock() {
                t.2.insert(ticket);
            }
            this.slots.1.notify_all();
            false
        };
        let Ok(mut s) = self.slots.0.lock() else {
            return false;
        };
        loop {
            let head = self
                .tickets
                .lock()
                .map(|mut t| Self::at_head(&mut t, ticket))
                .unwrap_or(false);
            if head && s.0 < s.1 {
                break;
            }
            if cancel.load(Ordering::Relaxed) {
                drop(s);
                return give_up(self);
            }
            let (guard, _) = match self
                .slots
                .1
                .wait_timeout(s, std::time::Duration::from_millis(50))
            {
                Ok(x) => x,
                Err(_) => return false,
            };
            s = guard;
        }
        if cancel.load(Ordering::Relaxed) {
            drop(s);
            return give_up(self);
        }
        s.0 += 1;
        if let Ok(mut t) = self.tickets.lock() {
            t.0 += 1; // this ticket is served; the next one may start
        }
        drop(s);
        self.slots.1.notify_all();
        if let Some(flag) = running_now {
            flag.store(true, Ordering::Relaxed);
        }
        true
    }

    /// Give a worker slot back.
    pub fn release_slot(&self) {
        if let Ok(mut s) = self.slots.0.lock() {
            s.0 = s.0.saturating_sub(1);
        }
        self.slots.1.notify_all();
    }

    /// Store a detached job's envelope for `jobs.result` (keeps the 64 most recent).
    pub fn finish(&self, id: u64, envelope: serde_json::Value) {
        if let Ok(mut f) = self.finished.lock() {
            f.insert(id, envelope);
            while f.len() > 64 {
                let oldest = f.keys().next().copied();
                if let Some(k) = oldest {
                    f.remove(&k);
                }
            }
        }
    }

    /// State of a job: running (with progress), done (envelope handed over once) or unknown.
    pub fn result(&self, id: u64) -> JobResult {
        if let Some(info) = self.list().into_iter().find(|j| j.id == id) {
            return JobResult {
                id,
                state: "running".into(),
                info: Some(info),
                response: None,
            };
        }
        let response = self.finished.lock().ok().and_then(|mut f| f.remove(&id));
        JobResult {
            id,
            state: if response.is_some() {
                "done"
            } else {
                "unknown"
            }
            .into(),
            info: None,
            response,
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
                    state: if e.running_now.load(Ordering::Relaxed) {
                        "running"
                    } else {
                        "queued"
                    }
                    .into(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_removes_the_job_even_when_the_op_panics() {
        let jobs = Jobs::default();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let guard = jobs.start("graph.evaluate");
            (guard.ctl.progress)(1, 3, "jd.noise.fbm@1");
            let listed = jobs.list();
            assert_eq!((listed.len(), listed[0].done, listed[0].total), (1, 1, 3));
            assert_eq!(listed[0].current, "jd.noise.fbm@1");
            panic!("kernel blew up");
        }));
        assert!(result.is_err());
        assert!(jobs.list().is_empty(), "the guard dropped during unwinding");
        assert!(!jobs.cancel(1), "unknown ids are not running");
        let g = jobs.start("x");
        assert!(jobs.cancel(g.id));
        assert!(g.ctl.cancel.load(Ordering::Relaxed));
        assert!(jobs.list()[0].cancelling);
    }
}
