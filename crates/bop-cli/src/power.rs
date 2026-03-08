use std::time::Duration;

/// System power state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepState {
    Awake,
    #[allow(dead_code)] // Used in tests; will be used in IOKit implementation (subtask-1-2)
    Sleeping,
}

/// Spawns a power state watcher on a dedicated OS thread.
/// Returns a watch receiver that subscribers can clone to monitor power state changes.
///
/// **Critical constraint (macOS)**: IORegisterForSystemPower and the IOKit run loop
/// MUST run on a dedicated OS thread (not a tokio task). Tokio tasks can be starved
/// under load, and if IOAllowPowerChange is not called promptly after
/// kIOMessageSystemWillSleep, macOS will hang the sleep transition for the full
/// kernel timeout (~30s).
///
/// On macOS: Uses IOKit to subscribe to system power notifications.
/// On Linux: Stub implementation (feature-gated for future zbus integration).
/// On other platforms: No-op (always returns Awake state).
pub fn spawn_power_watcher() -> tokio::sync::watch::Receiver<SleepState> {
    let (tx, rx) = tokio::sync::watch::channel(SleepState::Awake);

    #[cfg(target_os = "macos")]
    spawn_macos_watcher(tx);

    #[cfg(target_os = "linux")]
    spawn_linux_watcher(tx);

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        // No-op for other platforms
        let _ = tx;
    }

    rx
}

#[cfg(target_os = "macos")]
fn spawn_macos_watcher(_tx: tokio::sync::watch::Sender<SleepState>) {
    std::thread::spawn(move || {
        // IOKit integration will be added in subtask-1-2 after core-foundation dependency
        // For now, this is a placeholder that ensures the dedicated thread pattern is correct.
        //
        // The IOKit implementation will:
        // 1. Register for system power notifications via IORegisterForSystemPower
        // 2. Run a CFRunLoop on this dedicated OS thread
        // 3. On kIOMessageSystemWillSleep:
        //    - _tx.send(SleepState::Sleeping)
        //    - Call IOAllowPowerChange within 8s deadline
        // 4. On kIOMessageSystemHasPoweredOn:
        //    - _tx.send(SleepState::Awake)

        eprintln!("[power] macOS power watcher spawned (IOKit integration pending)");

        // Keep thread alive for now
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    });
}

#[cfg(target_os = "linux")]
fn spawn_linux_watcher(tx: tokio::sync::watch::Sender<SleepState>) {
    std::thread::spawn(move || {
        // Linux stub implementation
        // Future: subscribe to logind D-Bus PrepareForSleep signal using zbus
        // For now, retry heuristics (exit code 75) provide sufficient resilience
        eprintln!("[power] Linux power watcher spawned (stub - D-Bus integration pending)");

        // Keep thread alive
        loop {
            std::thread::sleep(Duration::from_secs(3600));
            let _ = &tx; // Silence unused warning
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleep_state_basics() {
        assert_eq!(SleepState::Awake, SleepState::Awake);
        assert_ne!(SleepState::Awake, SleepState::Sleeping);
    }

    #[test]
    fn spawn_watcher_returns_receiver() {
        let rx = spawn_power_watcher();
        // Should start in Awake state
        assert_eq!(*rx.borrow(), SleepState::Awake);
    }

    #[tokio::test]
    async fn receiver_can_be_cloned() {
        let rx = spawn_power_watcher();
        let rx2 = rx.clone();
        assert_eq!(*rx.borrow(), *rx2.borrow());
    }

    #[tokio::test]
    async fn watch_channel_semantics() {
        // Verify watch channel behavior without actual platform events
        let (tx, rx) = tokio::sync::watch::channel(SleepState::Awake);
        assert_eq!(*rx.borrow(), SleepState::Awake);

        tx.send(SleepState::Sleeping).unwrap();
        assert_eq!(*rx.borrow(), SleepState::Sleeping);

        tx.send(SleepState::Awake).unwrap();
        assert_eq!(*rx.borrow(), SleepState::Awake);
    }
}
