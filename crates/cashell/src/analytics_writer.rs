//! Batched writer for the analytics collector (`POST /api/collect`).
//!
//! The collector used to open SQLite (and re-run migrations) on every request,
//! synchronously on a tokio worker. Now handlers only push into a bounded
//! channel; one dedicated thread owns a single connection and flushes events in
//! transactions of up to `MAX_BATCH` rows, or every `FLUSH_INTERVAL`.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::db::{AnalyticsEvent, Database};

/// Events buffered in memory before the collector starts shedding load.
const QUEUE_CAPACITY: usize = 20_000;
/// Rows per transaction.
const MAX_BATCH: usize = 500;
/// Longest an accepted event waits before it is written.
const FLUSH_INTERVAL: Duration = Duration::from_millis(1000);

/// Why an event could not be queued.
#[derive(Debug, PartialEq, Eq)]
pub enum SubmitError {
    /// Queue is full: the writer is behind. Caller should answer 503.
    Busy,
    /// Writer thread is gone (database could not be opened).
    Unavailable,
}

static SENDER: OnceLock<Option<SyncSender<AnalyticsEvent>>> = OnceLock::new();

fn sender() -> Option<&'static SyncSender<AnalyticsEvent>> {
    SENDER
        .get_or_init(|| {
            let db = match Database::open() {
                Ok(db) => db,
                Err(e) => {
                    eprintln!("analytics writer: cannot open database: {e:#}");
                    return None;
                }
            };
            let (tx, rx) = mpsc::sync_channel(QUEUE_CAPACITY);
            std::thread::Builder::new()
                .name("cas-analytics-writer".into())
                .spawn(move || writer_loop(db, rx))
                .ok()?;
            Some(tx)
        })
        .as_ref()
}

/// Queue one event for writing. Never blocks.
pub fn submit(event: AnalyticsEvent) -> Result<(), SubmitError> {
    let tx = sender().ok_or(SubmitError::Unavailable)?;
    match tx.try_send(event) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => Err(SubmitError::Busy),
        Err(TrySendError::Disconnected(_)) => Err(SubmitError::Unavailable),
    }
}

fn writer_loop(db: Database, rx: Receiver<AnalyticsEvent>) {
    let mut batch: Vec<AnalyticsEvent> = Vec::with_capacity(MAX_BATCH);
    loop {
        // Block until the first event of the next batch arrives.
        match rx.recv() {
            Ok(ev) => batch.push(ev),
            Err(_) => return,
        }
        let deadline = Instant::now() + FLUSH_INTERVAL;
        let mut closed = false;
        while batch.len() < MAX_BATCH {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match rx.recv_timeout(left) {
                Ok(ev) => batch.push(ev),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    closed = true;
                    break;
                }
            }
        }
        flush(&db, &mut batch);
        if closed {
            return;
        }
    }
}

fn flush(db: &Database, batch: &mut Vec<AnalyticsEvent>) {
    if let Err(e) = db.record_analytics_events_batch(batch) {
        eprintln!(
            "analytics writer: failed to write {} events: {e:#}",
            batch.len()
        );
    }
    batch.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_insert_writes_all_rows() {
        let db = Database::open_in_memory().unwrap();
        let events: Vec<AnalyticsEvent> = (0..1200)
            .map(|i| AnalyticsEvent {
                website_id: "site".into(),
                session_id: format!("s{}", i % 7),
                url: format!("/p/{i}"),
                event_type: "pageview".into(),
                ..Default::default()
            })
            .collect();
        for chunk in events.chunks(MAX_BATCH) {
            db.record_analytics_events_batch(chunk).unwrap();
        }
        let n: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM analytics_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1200);
    }

    #[test]
    fn writer_loop_flushes_on_disconnect() {
        let path =
            std::env::temp_dir().join(format!("cas-analytics-writer-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let db = Database::open_path(&path).unwrap();
        let (tx, rx) = mpsc::sync_channel(16);
        let handle = std::thread::spawn(move || writer_loop(db, rx));
        for i in 0..5 {
            tx.send(AnalyticsEvent {
                website_id: "w".into(),
                url: format!("/{i}"),
                event_type: "pageview".into(),
                ..Default::default()
            })
            .unwrap();
        }
        drop(tx);
        handle.join().unwrap();
        let check = Database::open_path(&path).unwrap();
        let n: i64 = check
            .conn()
            .query_row("SELECT COUNT(*) FROM analytics_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 5);
        drop(check);
        let _ = std::fs::remove_file(&path);
    }
}
