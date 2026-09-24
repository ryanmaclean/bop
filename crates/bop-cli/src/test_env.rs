//! Process-wide guard for unit tests that mutate environment variables.
//!
//! `cargo test` runs every unit test of the `bop` binary in one process on
//! many threads. Each provider module used to guard `HOME` with its own mutex
//! (or none at all), so `claude`, `codex`, `gemini` and `ollama` tests could
//! swap `HOME` underneath each other and fail intermittently — this is what
//! failed Auto-Claude's review of spec 032. Every test that touches `HOME`
//! (or any other shared env var) must hold [`lock`] or a [`HomeGuard`].

use std::ffi::OsString;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Serialise env-mutating tests. Poisoning is ignored so one failing test
/// does not cascade into every later env test.
pub fn lock() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Points `HOME` at `path` while held; restores the previous value on drop
/// (including when the test panics).
pub struct HomeGuard {
    saved: Option<OsString>,
    _lock: MutexGuard<'static, ()>,
}

impl HomeGuard {
    pub fn set(path: &Path) -> Self {
        let lock = lock();
        let saved = std::env::var_os("HOME");
        std::env::set_var("HOME", path);
        Self { saved, _lock: lock }
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match self.saved.take() {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_guard_restores_previous_home() {
        let before = std::env::var_os("HOME");
        let td = tempfile::tempdir().unwrap();
        {
            let _g = HomeGuard::set(td.path());
            assert_eq!(
                std::env::var_os("HOME").as_deref(),
                Some(td.path().as_os_str())
            );
        }
        assert_eq!(std::env::var_os("HOME"), before);
    }
}
