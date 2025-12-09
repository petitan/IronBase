// Helper Utilities - Reusable Functions
// This script should be saved as a dependency for other scripts
// Dependencies: none

// Math helpers
fn add(a, b) { a + b }
fn subtract(a, b) { a - b }
fn multiply(a, b) { a * b }
fn safe_divide(a, b) { if b == 0 { 0 } else { a / b } }

// Currency formatting
fn format_currency(amount) {
    `$${amount}`
}

fn format_percentage(value) {
    `${value * 100}%`
}

// Tax calculation
fn calculate_tax(amount, rate) {
    amount * rate
}

fn calculate_with_tax(amount, rate) {
    amount + calculate_tax(amount, rate)
}

// Date helpers (returns current timestamp as string)
fn get_current_date() {
    // Returns ISO format date string
    "2024-01-01T00:00:00Z"  // Note: In real Rhai, you'd use chrono bindings
}

// Array helpers
fn sum_array(arr) {
    let total = 0;
    for item in arr {
        total += item;
    }
    total
}

fn average(arr) {
    if arr.len() == 0 { return 0; }
    safe_divide(sum_array(arr), arr.len())
}

fn find_max(arr) {
    if arr.len() == 0 { return 0; }
    let max = arr[0];
    for item in arr {
        if item > max { max = item; }
    }
    max
}

fn find_min(arr) {
    if arr.len() == 0 { return 0; }
    let min = arr[0];
    for item in arr {
        if item < min { min = item; }
    }
    min
}

// Return confirmation that helpers are loaded
"Helper utilities loaded successfully"
