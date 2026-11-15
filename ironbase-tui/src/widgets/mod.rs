//! Shared widgets: one implementation per kind of control, used by every
//! modal (instead of each modal editing its own strings)

pub mod text_input;

pub use text_input::{InputOutcome, TextInput};
