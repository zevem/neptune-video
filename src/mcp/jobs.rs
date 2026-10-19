//! Long work the server does for an agent: transcription, model downloads,
//! scene detection, loudness. Each runs on a thread of its own, reports a
//! fraction done and a message, can be cancelled, and ends with a result
//! or an error. Jobs never edit the project: what they find is returned to
//! the agent, which applies it with an ordinary tool.
//!
//! Exports are not here: they run in a render queue (the window's or the
//! server's), and the `job` tool reads them from there.
use super::errors::{ToolError, codes};
use schemars::JsonSchema;
use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

/// The most jobs running at once.
pub const MOST_RUNNING: usize = 4;
/// The most finished jobs kept for their results.
pub const MOST_KEPT: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn finished(self) -> bool {
        self != Self::Running
    }
}

/// A job as the `job` tool reports it.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct JobInfo {
    pub job_id: Uuid,
    /// `transcribe`, `model_install`, `scenes`, `loudness` or `export`.
    pub kind: String,
    pub state: JobState,
    /// From 0 to 1.
    pub progress: f32,
    pub message: String,
    /// What the job found, once it is done.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ToolError>,
    /// Seconds since the job started.
    pub elapsed_seconds: f64,
    /// When it started, in seconds since 1970 (not known for an export
    /// the window queued).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
}

struct Job {
    info: JobInfo,
    started: Instant,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct Registry {
    jobs: VecDeque<Job>,
    generation: u64,
}

/// What a running job reports through.
#[derive(Clone)]
pub struct Progress {
    id: Uuid,
    jobs: Jobs,
    cancel: Arc<AtomicBool>,
}

impl Progress {
    pub fn set(&self, done: f32, message: impl Into<String>) {
        let message = message.into();
        self.jobs.update(self.id, |job| {
            job.info.progress = done.clamp(0.0, 1.0);
            job.info.message = message;
        });
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    pub fn flag(&self) -> &AtomicBool {
        &self.cancel
    }
}

#[derive(Clone, Default)]
pub struct Jobs {
    registry: Arc<Mutex<Registry>>,
    changed: Arc<tokio::sync::Notify>,
}

impl Jobs {
    fn lock(&self) -> std::sync::MutexGuard<'_, Registry> {
        self.registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Changes a job; waiters are told only when its state, progress,
    /// message, result or error moved.
    fn update(&self, id: Uuid, change: impl FnOnce(&mut Job)) {
        {
            let mut registry = self.lock();
            let Some(job) = registry.jobs.iter_mut().find(|job| job.info.job_id == id) else {
                return;
            };
            let before = job.info.clone();
            change(job);
            job.info.elapsed_seconds = job.started.elapsed().as_secs_f64();
            let mut after = job.info.clone();
            after.elapsed_seconds = before.elapsed_seconds;
            if after == before {
                return;
            }
            registry.generation += 1;
        }
        self.changed.notify_waiters();
    }

    /// Asks every running job to stop, and waits up to `within` for them
    /// to end: a download's or a transcription's helper processes are
    /// stopped by their jobs, not left behind.
    pub fn cancel_all(&self, within: Duration) {
        {
            let registry = self.lock();
            for job in &registry.jobs {
                job.cancel.store(true, Ordering::Release);
            }
        }
        let until = Instant::now() + within;
        while Instant::now() < until && self.list().iter().any(|job| job.state == JobState::Running)
        {
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Starts `work` on a thread of its own and answers its id at once.
    pub fn start(
        &self,
        kind: &str,
        message: &str,
        work: impl FnOnce(&Progress) -> Result<serde_json::Value, ToolError> + Send + 'static,
    ) -> Result<JobInfo, ToolError> {
        let id = Uuid::new_v4();
        let cancel = Arc::new(AtomicBool::new(false));
        let info = JobInfo {
            job_id: id,
            kind: kind.to_owned(),
            state: JobState::Running,
            progress: 0.0,
            message: message.to_owned(),
            result: None,
            error: None,
            elapsed_seconds: 0.0,
            started_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|since| since.as_secs()),
        };
        {
            let mut registry = self.lock();
            let running = (registry.jobs.iter())
                .filter(|job| job.info.state == JobState::Running)
                .count();
            if running >= MOST_RUNNING {
                return Err(ToolError::new(
                    codes::BUSY,
                    format!("{MOST_RUNNING} jobs are running; wait for one or cancel one"),
                ));
            }
            registry.jobs.push_back(Job {
                info: info.clone(),
                started: Instant::now(),
                cancel: cancel.clone(),
            });
            // Finished jobs beyond the limit go, oldest first.
            while registry.jobs.len() > MOST_KEPT {
                let Some(index) = (registry.jobs.iter()).position(|job| job.info.state.finished())
                else {
                    break;
                };
                registry.jobs.remove(index);
            }
            registry.generation += 1;
        }
        let progress = Progress {
            id,
            jobs: self.clone(),
            cancel: cancel.clone(),
        };
        let spawned = thread::Builder::new()
            .name(format!("neptune-video-mcp-{kind}"))
            .spawn(move || {
                let outcome =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&progress)))
                        .unwrap_or_else(|_| {
                            Err(ToolError::new(codes::JOB_FAILED, "The job failed"))
                        });
                let cancelled = progress.cancelled();
                progress.jobs.update(id, |job| match outcome {
                    Ok(result) => {
                        job.info.state = JobState::Done;
                        job.info.progress = 1.0;
                        job.info.message = "Done".into();
                        job.info.result = Some(result);
                    }
                    Err(_) if cancelled => {
                        job.info.state = JobState::Cancelled;
                        job.info.message = "Cancelled".into();
                    }
                    Err(error) => {
                        job.info.state = JobState::Failed;
                        job.info.message = error.message.clone();
                        job.info.error = Some(error);
                    }
                });
            });
        if let Err(error) = spawned {
            self.update(id, |job| {
                job.info.state = JobState::Failed;
                job.info.message = format!("The job could not start: {error}");
            });
        }
        Ok(info)
    }

    pub fn get(&self, id: Uuid) -> Option<JobInfo> {
        let registry = self.lock();
        (registry.jobs.iter())
            .find(|job| job.info.job_id == id)
            .map(|job| {
                let mut info = job.info.clone();
                if !info.state.finished() {
                    info.elapsed_seconds = job.started.elapsed().as_secs_f64();
                }
                info
            })
    }

    pub fn list(&self) -> Vec<JobInfo> {
        let registry = self.lock();
        registry.jobs.iter().map(|job| job.info.clone()).collect()
    }

    /// Asks a job to stop; it ends `cancelled` once its work notices.
    pub fn cancel(&self, id: Uuid) -> Option<JobInfo> {
        {
            let registry = self.lock();
            let job = registry.jobs.iter().find(|job| job.info.job_id == id)?;
            job.cancel.store(true, Ordering::Release);
        }
        self.changed.notify_waiters();
        self.get(id)
    }

    pub fn generation(&self) -> u64 {
        self.lock().generation
    }

    /// Resolves when a job changes.
    pub async fn changed(&self) {
        self.changed.notified().await;
    }

    /// The result of a finished job of `kind`, for a tool that applies it.
    pub fn result(&self, id: Uuid, kind: &str) -> Result<serde_json::Value, ToolError> {
        let info = self
            .get(id)
            .ok_or_else(|| ToolError::not_found(format!("No job {id}")))?;
        if info.kind != kind {
            return Err(ToolError::invalid(format!(
                "Job {id} is a {} job, not {kind}",
                info.kind
            )));
        }
        match info.state {
            JobState::Done => info
                .result
                .ok_or_else(|| ToolError::new(codes::JOB_FAILED, "The job left no result")),
            JobState::Running => Err(ToolError::new(
                codes::BUSY,
                format!("Job {id} is still running; wait for it with job {{op: wait}}"),
            )),
            JobState::Failed | JobState::Cancelled => Err(ToolError::new(
                codes::JOB_FAILED,
                format!("Job {id} did not finish: {}", info.message),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn wait(jobs: &Jobs, id: Uuid) -> JobInfo {
        for _ in 0..500 {
            let info = jobs.get(id).unwrap();
            if info.state.finished() {
                return info;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("the job did not finish");
    }

    #[test]
    fn a_job_reports_progress_and_keeps_its_result() {
        let jobs = Jobs::default();
        let started = jobs
            .start("scenes", "Looking", |progress| {
                progress.set(0.5, "Half");
                Ok(serde_json::json!({"cuts": 3}))
            })
            .unwrap();
        let done = wait(&jobs, started.job_id);
        assert_eq!(done.state, JobState::Done);
        assert_eq!(done.progress, 1.0);
        assert_eq!(jobs.result(started.job_id, "scenes").unwrap()["cuts"], 3);
        assert!(jobs.result(started.job_id, "loudness").is_err());
    }

    #[test]
    fn a_cancelled_job_ends_cancelled_and_has_no_result() {
        let jobs = Jobs::default();
        let started = jobs
            .start("transcribe", "Listening", |progress| {
                while !progress.cancelled() {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(ToolError::new("cancelled", "stopped"))
            })
            .unwrap();
        jobs.cancel(started.job_id);
        let done = wait(&jobs, started.job_id);
        assert_eq!(done.state, JobState::Cancelled);
        assert!(jobs.result(started.job_id, "transcribe").is_err());
    }

    #[test]
    fn no_more_than_the_limit_run_at_once() {
        let jobs = Jobs::default();
        let stop = Arc::new(AtomicBool::new(false));
        for _ in 0..MOST_RUNNING {
            let stop = stop.clone();
            jobs.start("scenes", "", move |_| {
                while !stop.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(5));
                }
                Ok(serde_json::Value::Null)
            })
            .unwrap();
        }
        let refused = jobs.start("scenes", "", |_| Ok(serde_json::Value::Null));
        assert_eq!(refused.unwrap_err().code, codes::BUSY);
        stop.store(true, Ordering::Release);
    }
}
