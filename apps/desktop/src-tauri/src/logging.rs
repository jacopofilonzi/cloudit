//! Logging: `tracing` to a daily rolling file in the app's log directory (7 files kept)
//! and, in debug builds, to the console. `RUST_LOG` overrides the filter.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Layer, Registry};

type BoxedLayer = Box<dyn Layer<Registry> + Send + Sync>;

/// Starts logging; the returned guard must live as long as the app (it flushes).
pub fn init(log_dir: &Path) -> Option<WorkerGuard> {
    let mut layers: Vec<BoxedLayer> = Vec::new();
    if cfg!(debug_assertions) {
        layers.push(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .boxed(),
        );
    }
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("cloudit")
        .filename_suffix("log")
        .max_log_files(7)
        .build(log_dir);
    let guard = match appender {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            layers.push(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(writer)
                    .boxed(),
            );
            Some(guard)
        }
        Err(error) => {
            eprintln!("cloudit: file logging is off: {error}");
            None
        }
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::registry()
        .with(layers)
        .with(filter)
        .try_init();
    guard
}
