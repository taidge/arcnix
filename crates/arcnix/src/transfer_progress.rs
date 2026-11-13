//! Transient per-attempt timing. Logical completion is never used as network bytes.
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::*;

#[derive(Clone)]
pub(super) struct Meter {
    start: Instant,
    end: Option<Instant>,
    paused: Option<Instant>,
    waited: Duration,
    samples: VecDeque<(Duration, u64)>,
    actual: u64,
}
impl Meter {
    pub fn new(now: Instant) -> Self {
        Self {
            start: now,
            end: None,
            paused: None,
            waited: Duration::ZERO,
            samples: VecDeque::from([(Duration::ZERO, 0)]),
            actual: 0,
        }
    }
    fn active(&self, now: Instant) -> Duration {
        self.paused
            .unwrap_or(self.end.unwrap_or(now))
            .saturating_duration_since(self.start)
            .saturating_sub(self.waited)
    }
    pub fn observe(&mut self, bytes: u64, now: Instant) {
        let at = self.active(now);
        self.actual = self.actual.max(bytes);
        self.samples.push_back((at, self.actual));
        let cutoff = at.saturating_sub(Duration::from_secs(3));
        while self.samples.len() > 2 && self.samples[1].0 <= cutoff {
            self.samples.pop_front();
        }
    }
    pub fn pause(&mut self, now: Instant) {
        if self.paused.is_none() {
            self.paused = Some(now);
        }
    }
    pub fn resume(&mut self, now: Instant) {
        if let Some(at) = self.paused.take() {
            self.waited += now.saturating_duration_since(at);
        }
    }
    pub fn finish(&mut self, now: Instant) {
        if self.end.is_none() {
            self.resume(now);
            self.end = Some(now);
        }
    }
    fn rate(&self, now: Instant) -> Option<f64> {
        if self.paused.is_some() {
            return None;
        }
        let active = self.active(now);
        if self.end.is_some() {
            return (active.as_secs_f64() > 0.).then(|| self.actual as f64 / active.as_secs_f64());
        }
        if active < Duration::from_millis(250) {
            return None;
        }
        let last = self.samples.back()?;
        if active.saturating_sub(last.0) >= Duration::from_secs(3) {
            return Some(0.);
        }
        let cutoff = active.saturating_sub(Duration::from_secs(3));
        let base = self
            .samples
            .iter()
            .rev()
            .find(|s| s.0 <= cutoff)
            .or(self.samples.front())?;
        let seconds = active.saturating_sub(base.0).as_secs_f64();
        (seconds > 0.).then(|| self.actual.saturating_sub(base.1) as f64 / seconds)
    }
}
pub(super) fn bytes(value: u64) -> String {
    if value >= 1 << 30 {
        format!("{:.1} GiB", value as f64 / (1u64 << 30) as f64)
    } else if value >= 1 << 20 {
        format!("{:.1} MiB", value as f64 / (1u64 << 20) as f64)
    } else if value >= 1 << 10 {
        format!("{:.1} KiB", value as f64 / 1024.)
    } else {
        format!("{value} B")
    }
}
pub(super) fn percent(job: &TransferJob) -> f64 {
    if job.status == TransferStatus::Completed {
        return 100.;
    }
    if job.total_bytes == 0 {
        return 0.;
    }
    (job.transferred_bytes as f64 / job.total_bytes as f64 * 100.).clamp(0., 99.9)
}
fn duration(value: Duration) -> String {
    let s = value.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}
pub(super) fn details(s: &State, job: &TransferJob) -> String {
    let Some(m) = s.transfer_meters.get(&job.id) else {
        return String::new();
    };
    let rate = m.rate(s.transfer_now);
    let speed = rate
        .map(|v| format!("{}/s", bytes(v.max(0.).min(u64::MAX as f64) as u64)))
        .unwrap_or_else(|| "—".into());
    let elapsed = duration(
        m.end
            .unwrap_or(s.transfer_now)
            .saturating_duration_since(m.start),
    );
    let actual = bytes(m.actual);
    let timing = if job.status == TransferStatus::Running {
        let eta = if m.paused.is_some()
            || job.total_bytes == 0
            || job.transferred_bytes >= job.total_bytes
        {
            None
        } else {
            rate.filter(|v| *v > 0.).and_then(|v| {
                Duration::try_from_secs_f64((job.total_bytes - job.transferred_bytes) as f64 / v)
                    .ok()
            })
        };
        format!(
            "{speed} · {} · {}",
            i18n::translate(&format!("耗时 {elapsed}")),
            i18n::translate(&format!(
                "剩余 {}",
                eta.map(duration).unwrap_or_else(|| "—".into())
            ))
        )
    } else {
        format!(
            "{} · {}",
            i18n::translate(&format!("平均 {speed}")),
            i18n::translate(&format!("耗时 {elapsed}"))
        )
    };
    format!(
        "{timing} · {}",
        i18n::translate(&format!("实际传输 {actual}"))
    )
}
impl Ui {
    pub(super) fn transfer_tick(&self) {
        if self
            .state
            .with(|s| s.jobs.iter().any(|j| j.status == TransferStatus::Running))
        {
            self.state.revise(|s| s.transfer_now = Instant::now());
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zero_length_and_unpublished_transfers_have_finite_progress() {
        let mut job = TransferJob {
            id: String::new(),
            server_id: String::new(),
            direction: TransferDirection::Upload,
            local_path: PathBuf::new(),
            remote_path: String::new(),
            total_bytes: 0,
            transferred_bytes: 0,
            status: TransferStatus::Running,
        };
        assert_eq!(percent(&job), 0.);
        job.status = TransferStatus::Completed;
        assert_eq!(percent(&job), 100.);
        job.total_bytes = 100;
        job.transferred_bytes = 100;
        job.status = TransferStatus::Running;
        assert_eq!(percent(&job), 99.9);
        job.status = TransferStatus::Cancelled;
        assert!(percent(&job) < 100.);
        assert_eq!(bytes(1024), "1.0 KiB");
    }
    #[test]
    fn excludes_conflict_wait_and_decays_when_stalled() {
        let now = Instant::now();
        let mut m = Meter::new(now);
        m.observe(1000, now + Duration::from_secs(1));
        assert_eq!(m.rate(now + Duration::from_secs(1)), Some(1000.));
        m.pause(now + Duration::from_secs(1));
        assert_eq!(m.rate(now + Duration::from_secs(50)), None);
        m.resume(now + Duration::from_secs(51));
        m.observe(2000, now + Duration::from_secs(52));
        assert_eq!(m.rate(now + Duration::from_secs(52)), Some(1000.));
        assert_eq!(m.rate(now + Duration::from_secs(56)), Some(0.));
        m.finish(now + Duration::from_secs(52));
        assert_eq!(m.rate(now + Duration::from_secs(100)), Some(1000.));
    }
    #[test]
    fn skipped_progress_does_not_create_throughput_and_large_bytes_are_safe() {
        let now = Instant::now();
        let mut m = Meter::new(now);
        m.observe(0, now + Duration::from_secs(1));
        assert_eq!(m.rate(now + Duration::from_secs(1)), Some(0.));
        m.observe(u64::MAX, now + Duration::from_secs(2));
        assert!(m.rate(now + Duration::from_secs(2)).unwrap().is_finite());
    }
}
