use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

pub const DEFAULT_MAX_RESTARTS: usize = 3;
pub const DEFAULT_WINDOW_SECONDS: u64 = 60;
pub const DEFAULT_MAX_GAP_SECONDS: u64 = 300;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct BootLog {
    #[serde(default)]
    boots: Vec<f64>,
}

/// Persistent consecutive-boot circuit breaker to suppress crash-loop auto-resumes.
///
/// When an agent or turn triggers a fatal crash/SIGTERM, the supervisor (launchd /
/// systemd) automatically restarts the gateway. On boot, if restart-interrupted
/// sessions are pending, consecutive boots separated by at most 300s remain one
/// loop. A larger configured window widens that gap, never narrows it.
#[derive(Clone, Debug)]
pub struct RestartLoopGuard {
    path: PathBuf,
    max_restarts: usize,
    window_seconds: u64,
}

impl RestartLoopGuard {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self::with_config(path, DEFAULT_MAX_RESTARTS, DEFAULT_WINDOW_SECONDS)
    }

    pub fn with_config(path: impl Into<PathBuf>, max_restarts: usize, window_seconds: u64) -> Self {
        Self {
            path: path.into(),
            max_restarts,
            window_seconds,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load_boots(&self) -> Vec<f64> {
        let raw = match fs::read_to_string(&self.path) {
            Ok(content) => content,
            Err(_) => return Vec::new(),
        };
        match serde_json::from_str::<BootLog>(&raw) {
            Ok(log) => log.boots,
            Err(err) => {
                debug!(
                    path = %self.path.display(),
                    %err,
                    "restart_loop_guard: failed to parse JSON, starting empty"
                );
                Vec::new()
            }
        }
    }

    fn save_boots(&self, boots: &[f64]) {
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let data = BootLog {
            boots: boots.to_vec(),
        };
        if let Ok(raw) = serde_json::to_string_pretty(&data) {
            let tmp_path = self.path.with_extension("tmp");
            if fs::write(&tmp_path, raw).is_ok() {
                let _ = fs::rename(&tmp_path, &self.path);
            }
        }
    }

    fn boot_chain_at(&self, now: f64) -> Vec<f64> {
        let mut boots = self.load_boots();
        boots.sort_by(f64::total_cmp);
        let gap = self.window_seconds.max(DEFAULT_MAX_GAP_SECONDS) as f64;
        let mut previous = now;
        let start = boots
            .iter()
            .rposition(|&boot| {
                if boot > now {
                    return false;
                }
                if previous - boot > gap {
                    return true;
                }
                previous = boot;
                false
            })
            .map_or(0, |index| index + 1);
        boots.drain(..start);
        boots
    }

    /// Records a restart-interrupted boot, keeping only its consecutive chain.
    pub fn record_boot_at(&self, now: f64) -> Vec<f64> {
        let mut boots = self.boot_chain_at(now);
        boots.push(now);
        let retained = 50.max(self.max_restarts);
        boots.drain(..boots.len().saturating_sub(retained));
        self.save_boots(&boots);
        boots
    }

    /// Returns whether the unbroken boot chain has reached the configured limit.
    pub fn is_tripped_at(&self, now: f64) -> bool {
        self.max_restarts > 0 && self.boot_chain_at(now).len() >= self.max_restarts
    }

    /// Records this restart boot timestamp and checks if the breaker is tripped.
    /// Returns `true` if auto-resume should be SKIPPED.
    pub fn check_and_record_at(&self, now: f64) -> bool {
        let boots = self.record_boot_at(now);
        let tripped = if self.max_restarts > 0 {
            boots.len() >= self.max_restarts
        } else {
            false
        };
        if tripped {
            warn!(
                boots = boots.len(),
                max_gap_seconds = self.window_seconds.max(DEFAULT_MAX_GAP_SECONDS),
                max_restarts = self.max_restarts,
                path = %self.path.display(),
                "Restart-loop breaker TRIPPED: skipping auto-resume for consecutive interrupted boots"
            );
        }
        tripped
    }

    /// Uses the current wall clock time to record and check.
    pub fn check_and_record(&self) -> bool {
        let now = Utc::now().timestamp_millis() as f64 / 1000.0;
        self.check_and_record_at(now)
    }

    /// Clears the boot log file.
    pub fn clear(&self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_chain_trips_for_slow_cycles() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("slow.json");
        let verdicts: Vec<_> = [0.0, 150.0, 300.0]
            .into_iter()
            .map(|now| RestartLoopGuard::new(&path).check_and_record_at(now))
            .collect();
        assert_eq!(
            verdicts,
            [false, false, true],
            "slow persisted restart chain"
        );
        let guard = RestartLoopGuard::new(&path);
        assert!(
            guard.is_tripped_at(600.0),
            "the 300s boundary remains linked"
        );
        assert!(!guard.is_tripped_at(601.0), "a quiet 301s gap resets");
        assert!(!guard.check_and_record_at(601.0));
        assert_eq!(guard.load_boots(), [601.0]);

        let disabled = RestartLoopGuard::with_config(temp.path().join("off.json"), 0, 60);
        for now in 0..100 {
            assert!(!disabled.check_and_record_at(f64::from(now)));
        }
        assert!(!disabled.is_tripped_at(100.0));
        assert!(
            disabled.load_boots().len() <= 50,
            "bounded persisted history"
        );
    }

    #[test]
    fn test_sliding_window_trip_logic() {
        let temp = tempfile::tempdir().unwrap();
        let guard_file = temp.path().join("restart_loop.json");
        let guard = RestartLoopGuard::with_config(&guard_file, 3, 60);

        // Boot 1 at t=10.0: count=1, not tripped
        assert!(!guard.check_and_record_at(10.0));
        assert!(!guard.is_tripped_at(10.0));

        // Boot 2 at t=25.0: count=2, not tripped
        assert!(!guard.check_and_record_at(25.0));
        assert!(!guard.is_tripped_at(25.0));

        // Boot 3 at t=40.0: count=3 within 60s -> tripped!
        assert!(guard.check_and_record_at(40.0));
        assert!(guard.is_tripped_at(40.0));
    }

    #[test]
    fn test_quiet_gap_prunes_expired_boots() {
        let temp = tempfile::tempdir().unwrap();
        let guard_file = temp.path().join("restart_loop.json");
        let guard = RestartLoopGuard::with_config(&guard_file, 3, 60);

        // Boot 1 at t=0.0
        assert!(!guard.check_and_record_at(0.0));
        // Boot 2 at t=30.0
        assert!(!guard.check_and_record_at(30.0));

        // A 301s quiet gap ends the old chain despite the persisted history.
        assert!(!guard.check_and_record_at(331.0));
        assert!(!guard.is_tripped_at(331.0));
        assert_eq!(guard.load_boots(), [331.0]);

        assert!(!guard.check_and_record_at(400.0));
        assert!(guard.check_and_record_at(550.0));
        assert!(guard.is_tripped_at(550.0));
    }

    #[test]
    fn test_clear_removes_state() {
        let temp = tempfile::tempdir().unwrap();
        let guard_file = temp.path().join("restart_loop.json");
        let guard = RestartLoopGuard::with_config(&guard_file, 2, 60);

        assert!(!guard.check_and_record_at(10.0));
        assert!(guard.check_and_record_at(20.0)); // tripped
        assert!(guard_file.exists());

        guard.clear();
        assert!(!guard_file.exists());
        assert!(!guard.is_tripped_at(25.0));
    }

    #[test]
    fn test_corrupted_json_fails_open() {
        let temp = tempfile::tempdir().unwrap();
        let guard_file = temp.path().join("restart_loop.json");
        fs::write(&guard_file, "{ corrupted json !").unwrap();

        let guard = RestartLoopGuard::with_config(&guard_file, 3, 60);
        // Fails open -> starts fresh with 1 boot, not tripped
        assert!(!guard.check_and_record_at(10.0));
    }
}
