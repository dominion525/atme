use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// 別スレッドで、起動直後・interval ごと・合図を受けたときに work を動かす。
/// work が動いているあいだに届いた合図は 1 回にまとめる。合図の送り手がすべて無くなったら止まる。
pub fn spawn<F>(interval: Duration, mut work: F) -> (Sender<()>, JoinHandle<()>)
where
    F: FnMut() + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    let handle = thread::spawn(move || loop {
        work();
        match receiver.recv_timeout(interval) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        while receiver.try_recv().is_ok() {}
    });
    (sender, handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONG: Duration = Duration::from_secs(60);
    const WAIT: Duration = Duration::from_secs(1);
    const QUIET: Duration = Duration::from_millis(300);

    #[test]
    fn runs_on_start_and_on_trigger() {
        let (done, runs) = mpsc::channel();
        let (trigger, _handle) = spawn(LONG, move || done.send(()).unwrap());

        runs.recv_timeout(WAIT).unwrap();
        trigger.send(()).unwrap();
        runs.recv_timeout(WAIT).unwrap();
        assert!(runs.recv_timeout(QUIET).is_err());
    }

    #[test]
    fn runs_again_after_interval() {
        let (done, runs) = mpsc::channel();
        let (_trigger, _handle) = spawn(Duration::from_millis(50), move || done.send(()).unwrap());

        for _ in 0..3 {
            runs.recv_timeout(WAIT).unwrap();
        }
    }

    #[test]
    fn coalesces_triggers_sent_while_working() {
        let (done, runs) = mpsc::channel();
        let (trigger, _handle) = spawn(LONG, move || {
            done.send(()).unwrap();
            thread::sleep(Duration::from_millis(100));
        });

        runs.recv_timeout(WAIT).unwrap();
        for _ in 0..3 {
            trigger.send(()).unwrap();
        }
        runs.recv_timeout(WAIT).unwrap();
        assert!(runs.recv_timeout(QUIET).is_err());
    }

    #[test]
    fn stops_when_trigger_is_dropped() {
        let (trigger, handle) = spawn(LONG, || {});

        drop(trigger);

        handle.join().unwrap();
    }
}
