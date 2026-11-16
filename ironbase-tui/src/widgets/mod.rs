//! Shared widgets: one implementation per kind of control, used by every
//! modal (instead of each modal editing its own strings)

pub mod text_area;
pub mod text_input;

pub use text_area::TextArea;
pub use text_input::{Input, InputOutcome, TextInput};

/// `value` with the ASCII digits of `text` appended, as typed into a number
/// field (other characters are skipped; it stops growing at `usize::MAX`
/// instead of overflowing)
pub fn push_digits(value: usize, text: &str) -> usize {
    text.chars()
        .filter(char::is_ascii_digit)
        .filter_map(|c| c.to_digit(10))
        .fold(value, |n, d| {
            n.saturating_mul(10).saturating_add(d as usize)
        })
}

#[cfg(test)]
mod tests {
    use super::push_digits;

    #[test]
    fn test_push_digits() {
        assert_eq!(push_digits(7, "68"), 768);
        assert_eq!(push_digits(0, " 1,0x2\n"), 102);
        assert_eq!(push_digits(5, "abc٣"), 5);
        assert_eq!(push_digits(1, "99999999999999999999999"), usize::MAX);
    }
}
