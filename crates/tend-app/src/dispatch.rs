//! A fixed pool of workers with separate, bounded queues for reads and
//! writes, so the Home window's decisions never wait behind its reads (a
//! read of a Fleet host that does not answer can take seconds).

use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};

pub struct Dispatcher<J: Send + 'static> {
    reads: SyncSender<J>,
    writes: SyncSender<J>,
}

fn spawn<J: Send + 'static>(
    receiver: Arc<Mutex<Receiver<J>>>,
    handler: Arc<dyn Fn(J) + Send + Sync>,
) {
    std::thread::spawn(move || loop {
        let job = receiver
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .recv();
        let Ok(job) = job else { return };
        handler(job);
    });
}

impl<J: Send + 'static> Dispatcher<J> {
    /// `readers` workers on a queue of `read_queue` reads, `writers` on a
    /// queue of `write_queue` writes. The workers stop once the dispatcher
    /// is dropped and their queue is drained.
    pub fn new(
        readers: usize,
        read_queue: usize,
        writers: usize,
        write_queue: usize,
        handler: impl Fn(J) + Send + Sync + 'static,
    ) -> Self {
        let handler: Arc<dyn Fn(J) + Send + Sync> = Arc::new(handler);
        let (read_tx, read_rx) = sync_channel(read_queue);
        let (write_tx, write_rx) = sync_channel(write_queue);
        let read_rx = Arc::new(Mutex::new(read_rx));
        let write_rx = Arc::new(Mutex::new(write_rx));
        for _ in 0..readers {
            spawn(read_rx.clone(), handler.clone());
        }
        for _ in 0..writers {
            spawn(write_rx.clone(), handler.clone());
        }
        Self {
            reads: read_tx,
            writes: write_tx,
        }
    }

    /// Queue a job, or hand it back when its queue is full.
    pub fn submit(&self, job: J, write: bool) -> Result<(), J> {
        let queue = if write { &self.writes } else { &self.reads };
        queue.try_send(job).map_err(|error| match error {
            TrySendError::Full(job) | TrySendError::Disconnected(job) => job,
        })
    }
}

/// Whether a request changes something (and so takes the write queue).
#[must_use]
pub fn is_write(method: &str) -> bool {
    !matches!(method, "GET" | "HEAD")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    #[test]
    fn a_write_runs_while_every_reader_is_stuck() {
        let (done, finished) = channel::<&'static str>();
        let release = Arc::new(Mutex::new(()));
        let held = release.lock().unwrap();
        let gate = release.clone();
        let dispatcher = Dispatcher::new(3, 8, 1, 4, move |job: &'static str| {
            if job == "slow read" {
                drop(gate.lock().unwrap());
            }
            let _ = done.send(job);
        });
        for _ in 0..3 {
            dispatcher.submit("slow read", false).unwrap();
        }
        dispatcher.submit("queued read", false).unwrap();
        dispatcher.submit("decision", true).unwrap();
        // The decision runs while every reader is still held.
        assert_eq!(
            finished.recv_timeout(Duration::from_secs(10)).unwrap(),
            "decision"
        );
        drop(held);
        let mut rest: Vec<_> = (0..4)
            .map(|_| finished.recv_timeout(Duration::from_secs(10)).unwrap())
            .collect();
        rest.sort_unstable();
        assert_eq!(rest, ["queued read", "slow read", "slow read", "slow read"]);
    }

    #[test]
    fn a_full_queue_hands_the_job_back() {
        let (started, running) = channel::<u32>();
        let release = Arc::new(Mutex::new(()));
        let held = release.lock().unwrap();
        let gate = release.clone();
        let started = Mutex::new(started);
        let dispatcher = Dispatcher::new(1, 1, 1, 1, move |job: u32| {
            let _ = started.lock().unwrap().send(job);
            drop(gate.lock().unwrap());
        });
        dispatcher.submit(1, false).unwrap();
        // The worker took job 1 and blocks; the queue is empty again.
        assert_eq!(running.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
        dispatcher.submit(2, false).unwrap(); // fills the queue
        assert_eq!(dispatcher.submit(3, false), Err(3));
        drop(held);
    }

    #[test]
    fn reads_and_writes_by_method() {
        assert!(!is_write("GET"));
        assert!(!is_write("HEAD"));
        assert!(is_write("POST"));
        assert!(is_write("PUT"));
    }
}
