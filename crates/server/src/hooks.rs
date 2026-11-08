//! Pause points for tests that force interleavings (uploads vs the garbage collector,
//! two schedulers). In normal builds [`reached`] is an empty function the compiler
//! removes; under `cfg(test)` a test can [`arm`] a point for one key (a scope id, a job
//! name): the code that reaches it then waits until the test releases it.
//!
//! The points sit outside database transactions, so a paused task never holds a lock
//! (on SQLite that would block every other writer).

/// Where the code can be paused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Point {
    /// An upload stored its object and is about to insert the row (key: scope id).
    UploadStored,
    /// A commit verified an object without row and is about to adopt it (key: scope id).
    AdoptVerified,
    /// A commit has every row it needs and is about to insert the snapshot (key: scope
    /// id).
    CommitReady,
    /// The garbage collector deleted rows of a scope and is about to delete their
    /// objects (key: scope id).
    GcRowsDeleted,
    /// The orphan sweep found objects without rows in a scope and is about to delete
    /// them (key: scope id).
    SweepListed,
    /// A job holds its lease and is about to run (key: `<instance id>:<job name>`).
    JobRunning,
}

#[cfg(not(test))]
#[inline]
#[allow(clippy::unused_async)]
pub async fn reached(_point: Point, _key: &str) {}

#[cfg(test)]
pub use armed::*;

#[cfg(test)]
mod armed {
    use std::collections::HashMap;
    use std::sync::{Arc, LazyLock, Mutex};

    use tokio::sync::Notify;

    use super::Point;

    #[derive(Default)]
    struct Hook {
        reached: Notify,
        release: Notify,
    }

    type Registry = Mutex<HashMap<(Point, String), Arc<Hook>>>;

    static HOOKS: LazyLock<Registry> = LazyLock::new(|| Mutex::new(HashMap::new()));

    /// Pauses here if a test armed `point` for `key` (once).
    pub async fn reached(point: Point, key: &str) {
        let hook = HOOKS.lock().unwrap().remove(&(point, key.to_owned()));
        if let Some(hook) = hook {
            hook.reached.notify_one();
            hook.release.notified().await;
        }
    }

    /// An armed point: the next task reaching it waits for [`Armed::release`].
    pub struct Armed(Arc<Hook>);

    pub fn arm(point: Point, key: &str) -> Armed {
        let hook = Arc::new(Hook::default());
        HOOKS
            .lock()
            .unwrap()
            .insert((point, key.to_owned()), hook.clone());
        Armed(hook)
    }

    impl Armed {
        /// Waits until a task is paused at the point (at most 10 s).
        pub async fn reached(&self) {
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                self.0.reached.notified(),
            )
            .await
            .expect("the hook point was not reached");
        }

        /// Lets the paused task continue.
        pub fn release(&self) {
            self.0.release.notify_one();
        }
    }
}
