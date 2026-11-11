//! Running engine operations for the UI: registration (cancellation), throttled
//! progress events, the started / finished / status events every operation emits, and
//! the end of each operation reported to the automatic sync scheduler (and its status
//! to the notifications, which tell about new conflicts).

use std::future::Future;
use std::mem::Discriminant;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cloudit_client::{Compat, FolderStatus, OpContext, Phase, Progress, SyncOptions, SyncReport};
use tauri::{AppHandle, Manager};

use crate::auto;
use crate::compat;
use crate::error::{CommandError, ErrorDetails};
use crate::events::{
    FOLDER_OP, FOLDER_PROGRESS, FOLDER_STATUS, FolderOpEvent, FolderStatusEvent, OpAction, OpStage,
    emit,
};
use crate::notifications;
use crate::state::AppState;

/// Progress events are sent at most this often (a phase change always goes through).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(120);

/// What an operation yields, for the events.
pub trait Outcome: Clone + serde::Serialize {
    fn status(&self) -> &FolderStatus;
    fn report(&self) -> Option<SyncReport>;
}

impl Outcome for FolderStatus {
    fn status(&self) -> &FolderStatus {
        self
    }
    fn report(&self) -> Option<SyncReport> {
        None
    }
}

impl Outcome for SyncReport {
    fn status(&self) -> &FolderStatus {
        &self.status
    }
    fn report(&self) -> Option<SyncReport> {
        Some(self.clone())
    }
}

/// Throttle state: when the last event went out and which phase it reported.
type Last = Mutex<Option<(Instant, Discriminant<Phase>)>>;

/// A context whose progress goes to the UI as throttled events.
fn progress_context(app: &AppHandle) -> OpContext {
    let app = app.clone();
    let last: Last = Mutex::new(None);
    OpContext::new().with_progress(move |progress: &Progress| {
        let kind = std::mem::discriminant(&progress.phase);
        let due = match last.lock() {
            Ok(mut last) => {
                let due = last.is_none_or(|(at, previous)| {
                    previous != kind || at.elapsed() >= PROGRESS_INTERVAL
                });
                if due {
                    *last = Some((Instant::now(), kind));
                }
                due
            }
            Err(_) => true,
        };
        if due {
            emit(&app, FOLDER_PROGRESS, progress);
        }
    })
}

/// Runs `operation` on `folder` (started by the user) with progress, cancellation
/// (`cancel_op`) and events.
pub async fn run<T, F, Fut>(
    app: &AppHandle,
    folder: i64,
    action: OpAction,
    operation: F,
) -> Result<T, CommandError>
where
    T: Outcome,
    F: FnOnce(OpContext) -> Fut,
    Fut: Future<Output = cloudit_client::Result<T>>,
{
    run_as(app, folder, action, false, operation).await
}

/// An automatic sync run (the scheduler starts none while the folder's program runs).
pub async fn run_auto(app: &AppHandle, folder: i64) -> Result<SyncReport, CommandError> {
    let engine = app.state::<AppState>().engine.clone();
    run_as(app, folder, OpAction::Sync, true, |ctx| async move {
        engine.sync_with(folder, SyncOptions::default(), &ctx).await
    })
    .await
}

async fn run_as<T, F, Fut>(
    app: &AppHandle,
    folder: i64,
    action: OpAction,
    auto: bool,
    operation: F,
) -> Result<T, CommandError>
where
    T: Outcome,
    F: FnOnce(OpContext) -> Fut,
    Fut: Future<Output = cloudit_client::Result<T>>,
{
    let state = app.state::<AppState>();
    let guard = state.begin_op(folder);
    let ctx = progress_context(app).with_cancel(guard.token.clone());
    emit(
        app,
        FOLDER_OP,
        &FolderOpEvent {
            folder,
            action,
            auto,
            stage: OpStage::Started,
        },
    );
    let result = operation(ctx).await.map_err(CommandError::from);
    drop(guard);
    state.auto.send(auto::Input::OpFinished {
        folder,
        auto,
        ended: auto::ended_of(&result),
    });
    match &result {
        Ok(_) => compat::update(app, Compat::Ok),
        Err(CommandError {
            details: Some(ErrorDetails::Compat { compat: learned }),
            ..
        }) => compat::update(app, learned.clone()),
        Err(_) => {}
    }
    match &result {
        Ok(outcome) => {
            notifications::observe(app, folder, outcome.status().state);
            emit(
                app,
                FOLDER_STATUS,
                &FolderStatusEvent {
                    folder,
                    status: Some(outcome.status().clone()),
                    error: None,
                },
            );
            emit(
                app,
                FOLDER_OP,
                &FolderOpEvent {
                    folder,
                    action,
                    auto,
                    stage: OpStage::Finished {
                        report: outcome.report().map(Box::new),
                        error: None,
                    },
                },
            );
        }
        Err(error) => {
            if auto {
                // The scheduler logs once per failure streak.
                tracing::debug!(folder, kind = ?error.kind, message = %error.message, "automatic sync failed");
            } else {
                tracing::warn!(folder, ?action, kind = ?error.kind, message = %error.message, "operation failed");
            }
            if action == OpAction::Status {
                emit(
                    app,
                    FOLDER_STATUS,
                    &FolderStatusEvent {
                        folder,
                        status: None,
                        error: Some(error.clone()),
                    },
                );
            }
            emit(
                app,
                FOLDER_OP,
                &FolderOpEvent {
                    folder,
                    action,
                    auto,
                    stage: OpStage::Finished {
                        report: None,
                        error: Some(error.clone()),
                    },
                },
            );
        }
    }
    result
}
