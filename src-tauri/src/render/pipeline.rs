//! One sequential producer + one analysis worker. At most two outstanding
//! analysis reservations (active/queued, or producer currently rendering).
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, RecvTimeoutError, TrySendError},
};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Event {
    CancelledBeforeRender,
    CancelledBeforeAnalysis,
    AnalysisPanicked,
}

pub(super) fn run<R: Send>(
    count: usize,
    cancel: &AtomicBool,
    mut render: impl FnMut(usize) -> Option<R>,
    mut analyze: impl FnMut(usize, R) + Send,
    event: impl Fn(usize, Event) + Sync,
) -> Result<(), String> {
    std::thread::scope(|scope| {
        let (sender, receiver) = mpsc::sync_channel::<(usize, R)>(1);
        let (slots, available) = mpsc::sync_channel(2);
        slots.send(()).map_err(|e| e.to_string())?;
        slots.send(()).map_err(|e| e.to_string())?;
        let return_slot = slots.clone();
        let notify = &event;
        let worker = std::thread::Builder::new()
            .name("1ce-key-worker".into())
            .spawn_scoped(scope, move || {
                while let Ok((index, audio)) = receiver.recv() {
                    if cancel.load(Ordering::Acquire) {
                        notify(index, Event::CancelledBeforeAnalysis);
                    } else if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        analyze(index, audio)
                    }))
                    .is_err()
                    {
                        notify(index, Event::AnalysisPanicked);
                    }
                    // Never block shutdown while returning a reservation.
                    let _ = return_slot.try_send(());
                }
            })
            .map_err(|e| format!("Cannot start key worker: {e}"))?;
        struct PanicCancel<'a>(&'a AtomicBool);
        impl Drop for PanicCancel<'_> {
            fn drop(&mut self) {
                if std::thread::panicking() {
                    self.0.store(true, Ordering::Release);
                }
            }
        }
        let _panic_cancel = PanicCancel(cancel);
        let produced = (|| {
            for index in 0..count {
                // Reserve space BEFORE rendering: a slow analyzer cannot create
                // an unbounded queue of already-rendered projects.
                loop {
                    if cancel.load(Ordering::Acquire) {
                        for pending in index..count {
                            event(pending, Event::CancelledBeforeRender);
                        }
                        return Ok(());
                    }
                    match available.recv_timeout(Duration::from_millis(20)) {
                        Ok(()) => break,
                        Err(RecvTimeoutError::Timeout) => {
                            if worker.is_finished() {
                                return Err("Key worker stopped before the batch finished".into());
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => {
                            return Err("Key worker reservations disconnected".into())
                        }
                    }
                }
                if cancel.load(Ordering::Acquire) {
                    for pending in index..count {
                        event(pending, Event::CancelledBeforeRender);
                    }
                    return Ok(());
                }
                let Some(audio) = render(index) else {
                    let _ = slots.try_send(());
                    continue;
                };
                let mut task = (index, audio);
                loop {
                    if cancel.load(Ordering::Acquire) {
                        event(index, Event::CancelledBeforeAnalysis);
                        for pending in index + 1..count {
                            event(pending, Event::CancelledBeforeRender);
                        }
                        return Ok(());
                    }
                    match sender.try_send(task) {
                        Ok(()) => break,
                        Err(TrySendError::Full(returned)) => {
                            task = returned;
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(TrySendError::Disconnected(_)) => {
                            return Err("Key worker disconnected; validated audio retained".into())
                        }
                    }
                }
            }
            Ok(())
        })();
        // Drain/join even after producer errors. No old worker may update the
        // shared job list after running=false permits a new batch to start.
        if produced.is_err() {
            cancel.store(true, Ordering::Release);
        }
        drop(sender);
        let joined = worker
            .join()
            .map_err(|_| "Key worker stopped unexpectedly".to_string());
        produced.and(joined)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    #[test]
    fn render_overlaps_analysis_but_backpressure_and_result_order_are_preserved() {
        let cancel = AtomicBool::new(false);
        let events = Arc::new(Mutex::new(Vec::new()));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (rendered_tx, rendered_rx) = mpsc::channel();
        let active = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let events_ref = &events;
            let active_ref = &active;
            let cancel_ref = &cancel;
            let batch = scope.spawn(move || {
                run(
                    3,
                    cancel_ref,
                    |index| {
                        events_ref.lock().unwrap().push(("render", index));
                        rendered_tx.send(index).unwrap();
                        Some(index)
                    },
                    move |index, audio| {
                        assert_eq!(index, audio);
                        assert!(!active_ref.swap(true, Ordering::AcqRel));
                        if index == 0 {
                            started_tx.send(()).unwrap();
                            release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                        }
                        events_ref.lock().unwrap().push(("done", index));
                        active_ref.store(false, Ordering::Release);
                    },
                    |_, event| panic!("Unexpected {event:?}"),
                )
            });
            started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            assert_eq!(rendered_rx.recv_timeout(Duration::from_secs(3)).unwrap(), 0);
            assert_eq!(rendered_rx.recv_timeout(Duration::from_secs(3)).unwrap(), 1);
            assert!(
                rendered_rx
                    .recv_timeout(Duration::from_millis(100))
                    .is_err(),
                "Third render started while both analysis reservations were occupied"
            );
            release_tx.send(()).unwrap();
            batch.join().unwrap().unwrap();
        });
        assert!(!active.load(Ordering::Acquire));
        let events = events.lock().unwrap();
        assert!(
            events.iter().position(|e| *e == ("render", 1)).unwrap()
                < events.iter().position(|e| *e == ("done", 0)).unwrap()
        );
        assert!(
            events.iter().position(|e| *e == ("done", 0)).unwrap()
                < events.iter().position(|e| *e == ("render", 2)).unwrap()
        );
        assert_eq!(events.iter().filter(|(kind, _)| *kind == "done").count(), 3);
    }
    #[test]
    fn cancellation_drains_queue_and_joins_active_analysis() {
        let cancel = AtomicBool::new(false);
        let active = AtomicBool::new(false);
        let events = Mutex::new(Vec::new());
        let result = run(
            5,
            &cancel,
            |index| {
                if index == 1 {
                    cancel.store(true, Ordering::Release);
                }
                Some(index)
            },
            |_, _| {
                active.store(true, Ordering::Release);
                while !cancel.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                active.store(false, Ordering::Release);
            },
            |index, event| events.lock().unwrap().push((index, event)),
        );
        assert!(result.is_ok());
        assert!(!active.load(Ordering::Acquire));
        let events = events.lock().unwrap();
        assert!(events.contains(&(1, Event::CancelledBeforeAnalysis)));
        for index in 2..5 {
            assert!(events.contains(&(index, Event::CancelledBeforeRender)));
        }
    }
    #[test]
    fn failed_render_never_enters_analysis_and_analysis_panic_does_not_lose_other_results() {
        let events = Mutex::new(Vec::new());
        let completed = Mutex::new(Vec::new());
        run(
            4,
            &AtomicBool::new(false),
            |index| if index == 1 { None } else { Some(index) },
            |index, _| {
                if index == 0 {
                    panic!("simulated DSP panic");
                }
                completed.lock().unwrap().push(index);
            },
            |index, event| events.lock().unwrap().push((index, event)),
        )
        .unwrap();
        assert_eq!(*completed.lock().unwrap(), vec![2, 3]);
        assert_eq!(*events.lock().unwrap(), vec![(0, Event::AnalysisPanicked)]);
    }
    #[test]
    fn producer_panic_cancels_and_joins_worker() {
        let cancel = AtomicBool::new(false);
        let active = AtomicBool::new(false);
        let result = std::panic::catch_unwind(|| {
            run(
                3,
                &cancel,
                |index| {
                    if index == 1 {
                        panic!("simulated renderer panic");
                    }
                    Some(index)
                },
                |_, _| {
                    active.store(true, Ordering::Release);
                    while !cancel.load(Ordering::Acquire) {
                        std::thread::yield_now();
                    }
                    active.store(false, Ordering::Release);
                },
                |_, _| {},
            )
        });
        assert!(result.is_err());
        assert!(cancel.load(Ordering::Acquire));
        assert!(!active.load(Ordering::Acquire));
    }
}
