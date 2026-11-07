//! Cooperative stop for the dispatcher's detached event receiver.
use std::{sync::{atomic::{AtomicBool, Ordering}, mpsc, Arc}, time::Duration};

pub(crate) struct WatcherStop(pub(crate) Arc<AtomicBool>);

impl Drop for WatcherStop {
    fn drop(&mut self) { self.0.store(true, Ordering::Release); }
}

pub(crate) fn next_watch_event<T>(rx: &mpsc::Receiver<T>, stop: &AtomicBool) -> Option<T> {
    while !stop.load(Ordering::Acquire) {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(value) => return Some(value),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_causes_idle_receiver_to_exit() {
        let stop = Arc::new(AtomicBool::new(false));
        let guard = WatcherStop(Arc::clone(&stop));
        let (_sender, receiver) = mpsc::channel::<()>();
        let (done_tx, done_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let worker_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            ready_tx.send(()).unwrap();
            assert!(next_watch_event(&receiver, &worker_stop).is_none());
            done_tx.send(()).unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(done_rx.recv_timeout(Duration::from_millis(150)), Err(mpsc::RecvTimeoutError::Timeout)));
        drop(guard);
        done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        thread.join().unwrap();
    }

    #[test]
    fn receives_events_and_stops_on_disconnection() {
        let stop = AtomicBool::new(false);
        let (sender, receiver) = mpsc::channel();
        sender.send(42).unwrap();
        assert_eq!(next_watch_event(&receiver, &stop), Some(42));
        drop(sender);
        assert_eq!(next_watch_event(&receiver, &stop), None);
    }
}
