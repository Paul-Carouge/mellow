//! Separate test binary: it installs a process-wide tracing subscriber, because sqlx logs from its
//! own worker threads, which a thread-local subscriber would not see.

use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use mellow_server::storage::{DatabaseKey, open_encrypted_database};
use tracing_subscriber::fmt::MakeWriter;

const KEY_BYTE: u8 = 0x5A;
const KEY_LENGTH_BYTES: usize = 32;

#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl io::Write for CapturedLogs {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test]
async fn opening_the_database_never_logs_the_key_even_at_trace_level() {
    let captured_logs = CapturedLogs::default();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(captured_logs.clone())
        .init();

    let directory = tempfile::tempdir().unwrap();
    let pool = open_encrypted_database(
        &directory.path().join("mellow.db"),
        &DatabaseKey::from_bytes([KEY_BYTE; KEY_LENGTH_BYTES]),
    )
    .await
    .unwrap();
    sqlx::query("SELECT 1").execute(&pool).await.unwrap();
    pool.close().await;
    tracing::info!("capture sentinel");

    let logs = String::from_utf8_lossy(&captured_logs.0.lock().unwrap()).into_owned();
    assert!(
        logs.contains("capture sentinel"),
        "the subscriber captured nothing, so the test proves nothing"
    );
    let key_hex = format!("{KEY_BYTE:02x}").repeat(KEY_LENGTH_BYTES);
    assert!(
        !logs.to_lowercase().contains(&key_hex),
        "the key leaked into logs"
    );
}
