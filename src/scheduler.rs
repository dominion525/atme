use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Fetching,
    /// 前回の取得から min_gap が経つのを待っている
    Waiting,
    Idle,
}

/// 別スレッドで、起動直後・interval ごと・合図を受けたときに work を動かす。
/// 前回の work が終わってから min_gap が経つまでは、合図を受けても待ってから動かす。
/// 待っているあいだや work が動いているあいだに届いた合図は 1 回にまとめる。
/// 状態が変わるたびに phase を呼ぶ。合図の送り手がすべて無くなったら止まる。
pub fn spawn<F, P>(
    interval: Duration,
    min_gap: Duration,
    mut work: F,
    phase: P,
) -> (Sender<()>, JoinHandle<()>)
where
    F: FnMut() + Send + 'static,
    P: Fn(Phase) + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    let handle = thread::spawn(move || loop {
        phase(Phase::Fetching);
        work();
        let finished = Instant::now();
        phase(Phase::Idle);
        match receiver.recv_timeout(interval) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        let elapsed = finished.elapsed();
        if elapsed < min_gap {
            phase(Phase::Waiting);
            if !wait(&receiver, min_gap - elapsed) {
                break;
            }
        }
        while receiver.try_recv().is_ok() {}
    });
    (sender, handle)
}

/// duration が経つまで待つ。そのあいだに届いた合図は捨てる。送り手が無くなったら false を返す
fn wait(receiver: &Receiver<()>, duration: Duration) -> bool {
    let deadline = Instant::now() + duration;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        match receiver.recv_timeout(deadline - now) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONG: Duration = Duration::from_secs(60);
    const NO_GAP: Duration = Duration::ZERO;
    const WAIT: Duration = Duration::from_secs(1);
    const QUIET: Duration = Duration::from_millis(300);

    #[test]
    fn runs_on_start_and_on_trigger() {
        let (done, runs) = mpsc::channel();
        let (trigger, _handle) = spawn(LONG, NO_GAP, move || done.send(()).unwrap(), |_| {});

        runs.recv_timeout(WAIT).unwrap();
        trigger.send(()).unwrap();
        runs.recv_timeout(WAIT).unwrap();
        assert!(runs.recv_timeout(QUIET).is_err());
    }

    #[test]
    fn runs_again_after_interval() {
        let (done, runs) = mpsc::channel();
        let (_trigger, _handle) = spawn(
            Duration::from_millis(50),
            NO_GAP,
            move || done.send(()).unwrap(),
            |_| {},
        );

        for _ in 0..3 {
            runs.recv_timeout(WAIT).unwrap();
        }
    }

    #[test]
    fn coalesces_triggers_sent_while_working() {
        // 1 回目の取得は、テストが合図を送り終えるまで終わらせない（時間に頼らずに「取得中に届いた」形を作る）
        let (done, runs) = mpsc::channel();
        let (release, gate) = mpsc::channel::<()>();
        let (trigger, _handle) = spawn(
            LONG,
            NO_GAP,
            move || {
                done.send(()).unwrap();
                let _ = gate.recv_timeout(WAIT);
            },
            |_| {},
        );

        runs.recv_timeout(WAIT).unwrap();
        for _ in 0..3 {
            trigger.send(()).unwrap();
        }
        // 送り手を手放すと、1 回目の取得の待ちが解け、2 回目からは待たない
        drop(release);
        runs.recv_timeout(WAIT).unwrap();
        assert!(runs.recv_timeout(QUIET).is_err());
    }

    #[test]
    fn waits_min_gap_after_previous_run() {
        let gap = Duration::from_millis(200);
        let (done, runs) = mpsc::channel();
        let (trigger, _handle) = spawn(
            LONG,
            gap,
            move || done.send(Instant::now()).unwrap(),
            |_| {},
        );

        let first = runs.recv_timeout(WAIT).unwrap();
        for _ in 0..3 {
            trigger.send(()).unwrap();
        }
        let second = runs.recv_timeout(WAIT).unwrap();

        assert!(second - first >= gap);
        assert!(runs.recv_timeout(QUIET).is_err());
    }

    #[test]
    fn reports_phases() {
        // 合図は 1 回目の取得が終わる前に送っておく。終わった直後に受け取るので、前回からの時間は必ず gap より短く、待ちを通る
        let gap = Duration::from_millis(300);
        let (report, phases) = mpsc::channel();
        let (release, gate) = mpsc::channel::<()>();
        let (trigger, _handle) = spawn(
            LONG,
            gap,
            move || {
                let _ = gate.recv();
            },
            move |phase| report.send(phase).unwrap(),
        );

        trigger.send(()).unwrap();
        drop(release);
        assert_eq!(phases.recv_timeout(WAIT).unwrap(), Phase::Fetching);
        assert_eq!(phases.recv_timeout(WAIT).unwrap(), Phase::Idle);
        assert_eq!(phases.recv_timeout(WAIT).unwrap(), Phase::Waiting);
        assert_eq!(phases.recv_timeout(WAIT).unwrap(), Phase::Fetching);
        assert_eq!(phases.recv_timeout(WAIT).unwrap(), Phase::Idle);
    }

    #[test]
    fn does_not_wait_when_gap_has_passed() {
        let gap = Duration::from_millis(50);
        let (report, phases) = mpsc::channel();
        let (trigger, _handle) = spawn(LONG, gap, || {}, move |phase| report.send(phase).unwrap());

        assert_eq!(phases.recv_timeout(WAIT).unwrap(), Phase::Fetching);
        assert_eq!(phases.recv_timeout(WAIT).unwrap(), Phase::Idle);
        thread::sleep(Duration::from_millis(100));
        trigger.send(()).unwrap();
        assert_eq!(phases.recv_timeout(WAIT).unwrap(), Phase::Fetching);
    }

    #[test]
    fn stops_when_trigger_is_dropped() {
        let (trigger, handle) = spawn(LONG, NO_GAP, || {}, |_| {});

        drop(trigger);

        handle.join().unwrap();
    }
}
