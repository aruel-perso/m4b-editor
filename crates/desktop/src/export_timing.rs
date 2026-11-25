use m4b_editor_core::ExportPhase;
use std::time::{Duration, Instant};

pub struct Timing {
    started: Instant,
    phase_started: Instant,
    phase_range: Option<(f32, f32)>,
    progress_at: Option<Instant>,
    progress: f32,
    finished: Option<Duration>,
}

impl Timing {
    pub fn new(now: Instant) -> Self {
        Self {
            started: now,
            phase_started: now,
            phase_range: None,
            progress_at: None,
            progress: 0.,
            finished: None,
        }
    }

    pub fn phase(&mut self, phase: &ExportPhase, now: Instant) {
        let range = match phase {
            ExportPhase::PreparingTrack(_) => Some((0., 0.45)),
            ExportPhase::Encoding => {
                Some((if self.phase_range.is_some() { 0.45 } else { 0. }, 0.95))
            }
            ExportPhase::VerifyingAudio => Some((0.95, 0.99)),
            _ => None,
        };
        if range != self.phase_range {
            self.phase_range = range;
            self.phase_started = now;
            self.progress_at = None;
        }
    }

    pub fn progress(&mut self, value: f32, now: Instant) {
        if value > self.progress {
            self.progress = value;
            self.progress_at = Some(now);
        }
    }

    pub fn finish(&mut self, now: Instant) {
        self.finished = Some(now.saturating_duration_since(self.started));
    }

    pub fn elapsed(&self, now: Instant) -> Duration {
        self.finished
            .unwrap_or_else(|| now.saturating_duration_since(self.started))
    }

    pub fn estimate(&self, now: Instant) -> Option<Duration> {
        let (start, end) = self.phase_range?;
        let elapsed = self.elapsed(now);
        let sample = self.progress_at?;
        let phase_elapsed = sample.saturating_duration_since(self.phase_started);
        let work = (self.progress.min(end) - start) / (end - start);
        if self.finished.is_some()
            || now.saturating_duration_since(sample) >= Duration::from_secs(10)
            || work < 0.02
            || phase_elapsed < Duration::from_secs(1)
        {
            return None;
        }
        let phase_remaining = phase_elapsed.as_secs_f64() * f64::from((1. - work) / work);
        // Use the observed overall rate as a provisional rate for later phases.
        // Their own measurements take over once those phases have started.
        let later = (0.99 - end) / self.progress;
        let sampled_elapsed = sample.saturating_duration_since(self.started).as_secs_f64();
        Duration::try_from_secs_f64(
            sampled_elapsed + phase_remaining + sampled_elapsed * f64::from(later),
        )
        .ok()
        .filter(|estimate| *estimate > elapsed)
    }
}

pub fn display(time: Duration) -> String {
    let seconds = time.as_secs();
    let minutes = seconds / 60;
    if minutes < 60 {
        format!("{minutes:02}:{:02}", seconds % 60)
    } else {
        format!("{}:{:02}:{:02}", minutes / 60, minutes % 60, seconds % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stalled_progress_keeps_total_fixed_then_unknown() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut timing = Timing::new(start);
        timing.phase(&ExportPhase::PreparingTrack(1), start);
        timing.progress(0.10, at(12));
        let initial = timing.estimate(at(12)).unwrap();
        assert_eq!(timing.estimate(at(17)), Some(initial));
        timing.progress(0.10, at(21));
        assert_eq!(timing.estimate(at(22)), None);
        timing.progress(0.15, at(23));
        assert!(timing.estimate(at(23)).is_some());
    }

    #[test]
    fn estimates_follow_phase_and_terminal_elapsed_is_frozen() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut timing = Timing::new(start);
        assert_eq!(timing.estimate(at(2)), None);
        timing.phase(&ExportPhase::PreparingTrack(1), at(2));
        timing.progress(0.05, at(2));
        assert_eq!(timing.estimate(at(2)), None);
        timing.progress(0.10, at(12));
        let initial = timing.estimate(at(12)).unwrap();
        assert!(initial > timing.elapsed(at(12)));
        timing.phase(&ExportPhase::PreparingTrack(2), at(12));
        assert_eq!(timing.estimate(at(12)), Some(initial));
        timing.progress(0.45, at(14));
        timing.phase(&ExportPhase::Encoding, at(14));
        assert_eq!(timing.estimate(at(14)), None);
        timing.progress(0.7, at(20));
        assert!(timing.estimate(at(20)).unwrap() > timing.elapsed(at(20)));
        timing.phase(&ExportPhase::CheckingOutput, at(21));
        assert_eq!(timing.estimate(at(23)), None);
        timing.phase(&ExportPhase::VerifyingAudio, at(23));
        timing.progress(0.97, at(26));
        assert!(timing.estimate(at(26)).unwrap() > timing.elapsed(at(26)));
        timing.finish(at(27));
        assert_eq!(timing.elapsed(at(30)), Duration::from_secs(27));
        assert_eq!(timing.estimate(at(30)), None);
    }
}
