//! Background work for the event loop
//!
//! A request that would block the screen runs as a task instead: it does the
//! network calls on its own (with a clone of the client) and returns an
//! [`Update`], a change to apply to the `App` once it is done. The event loop
//! keeps drawing and reading keys meanwhile, and applies the update when it
//! arrives.
//!
//! Each task has a [`Slot`]: a new task in a slot replaces (aborts) the one
//! running there, and an update is applied only if its task is still the
//! slot's current one, so a late answer to a superseded request is dropped.

use crate::app::App;
use std::collections::HashMap;
use std::future::Future;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// A change to the app, computed by a task
pub type Update = Box<dyn FnOnce(&mut App) + Send>;

/// What a task is for; one task per slot runs at a time
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    /// The document list's page and counts
    Documents,
    /// A request of the open modal (a search); closing the modal stops it
    Modal,
}

/// A finished task's update, tagged with its task
pub struct Finished {
    slot: Slot,
    id: u64,
    update: Update,
}

/// The running tasks
pub struct Tasks {
    tx: mpsc::UnboundedSender<Finished>,
    rx: mpsc::UnboundedReceiver<Finished>,
    running: HashMap<Slot, (u64, JoinHandle<()>)>,
    next_id: u64,
}

impl Default for Tasks {
    fn default() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx,
            running: HashMap::new(),
            next_id: 0,
        }
    }
}

impl Tasks {
    /// Run `task` in `slot`, replacing the task running there
    pub fn spawn<F>(&mut self, slot: Slot, task: F)
    where
        F: Future<Output = Update> + Send + 'static,
    {
        self.cancel(slot);
        self.next_id += 1;
        let id = self.next_id;
        let tx = self.tx.clone();
        let handle = tokio::spawn(async move {
            let update = task.await;
            // The receiver lives as long as the app: a send fails only on exit
            let _ = tx.send(Finished { slot, id, update });
        });
        self.running.insert(slot, (id, handle));
    }

    /// Stop the task in `slot` (its update, if already sent, is dropped)
    pub fn cancel(&mut self, slot: Slot) {
        if let Some((_, handle)) = self.running.remove(&slot) {
            handle.abort();
        }
    }

    /// Stop every task; true if there was any
    pub fn cancel_all(&mut self) -> bool {
        let any = !self.running.is_empty();
        for (_, (_, handle)) in self.running.drain() {
            handle.abort();
        }
        any
    }

    /// Whether a task runs in `slot`
    pub fn is_running(&self, slot: Slot) -> bool {
        self.running.contains_key(&slot)
    }

    /// Whether any task runs
    pub fn is_busy(&self) -> bool {
        !self.running.is_empty()
    }

    /// The next finished task (waits for one)
    pub async fn next_finished(&mut self) -> Finished {
        // `tx` lives in `self`, so the channel never closes
        self.rx.recv().await.expect("the sender lives in Tasks")
    }

    /// The update of a finished task, if it is still its slot's current task
    pub fn take_current(&mut self, finished: Finished) -> Option<Update> {
        match self.running.get(&finished.slot) {
            Some((id, _)) if *id == finished.id => {
                self.running.remove(&finished.slot);
                Some(finished.update)
            }
            _ => None,
        }
    }
}

impl App {
    /// Apply a finished task's update (a superseded or cancelled task's is dropped)
    pub fn apply_finished(&mut self, finished: Finished) {
        if let Some(update) = self.tasks.take_current(finished) {
            update(self);
        }
    }

    /// Wait for every running task and apply their updates (tests, and
    /// callers that need the result before going on)
    pub async fn settle(&mut self) {
        while self.tasks.is_busy() {
            let finished = self.tasks.next_finished().await;
            self.apply_finished(finished);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn set_status(text: &'static str) -> Update {
        Box::new(move |app: &mut App| app.set_status(text))
    }

    #[tokio::test]
    async fn test_a_newer_task_replaces_the_older() {
        let mut app = App::new(Config::default());
        let (release, wait) = tokio::sync::oneshot::channel::<()>();
        app.tasks.spawn(Slot::Documents, async move {
            let _ = wait.await;
            set_status("old")
        });
        app.tasks
            .spawn(Slot::Documents, async { set_status("new") });
        let _ = release.send(());
        app.settle().await;
        assert_eq!(app.status_message.as_deref(), Some("new"));
        assert!(!app.tasks.is_busy());
    }

    #[tokio::test]
    async fn test_a_cancelled_task_changes_nothing() {
        let mut app = App::new(Config::default());
        app.tasks
            .spawn(Slot::Documents, async { set_status("late") });
        // Finished, but not applied yet: cancelling drops its update
        tokio::task::yield_now().await;
        assert!(app.tasks.cancel_all());
        app.tasks
            .spawn(Slot::Documents, async { set_status("current") });
        app.settle().await;
        assert_eq!(app.status_message.as_deref(), Some("current"));
    }
}
