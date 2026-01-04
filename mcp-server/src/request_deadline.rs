use std::cell::Cell;
use std::time::Instant;

thread_local! {
    static REQUEST_DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
}

pub struct DeadlineGuard {
    previous: Option<Instant>,
}

impl Drop for DeadlineGuard {
    fn drop(&mut self) {
        REQUEST_DEADLINE.with(|cell| {
            cell.set(self.previous);
        });
    }
}

pub fn set_deadline(deadline: Option<Instant>) -> DeadlineGuard {
    let previous = REQUEST_DEADLINE.with(|cell| {
        let prev = cell.get();
        cell.set(deadline);
        prev
    });
    DeadlineGuard { previous }
}

pub fn current_deadline() -> Option<Instant> {
    REQUEST_DEADLINE.with(|cell| cell.get())
}
