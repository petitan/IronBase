//! Script-specific error types.
//!
//! Provides detailed error types for script execution and management.

use rhai::EvalAltResult;

use super::limits::DEFAULT_MAX_OPERATIONS;

/// Format a Rhai error for display to users.
///
/// Converts internal Rhai error types to user-friendly messages.
/// Specifically handles operation limit errors with clear explanations.
///
/// # Arguments
///
/// * `err` - The Rhai error to format
///
/// # Returns
///
/// A user-friendly error message string
///
/// # Example
///
/// ```rust,ignore
/// match engine.eval::<Dynamic>(code) {
///     Err(e) => {
///         let msg = format_rhai_error(&e);
///         return Err(McpError::ScriptError(msg));
///     }
///     ...
/// }
/// ```
pub fn format_rhai_error(err: &EvalAltResult) -> String {
    match err {
        EvalAltResult::ErrorTooManyOperations(_) => {
            format!(
                "Script exceeded maximum operation limit ({}). \
                Consider optimizing the script or increasing the limit.",
                DEFAULT_MAX_OPERATIONS
            )
        }
        EvalAltResult::ErrorTerminated(_, _) => {
            "Script was terminated (timeout or cancellation)".to_string()
        }
        EvalAltResult::ErrorRuntime(msg, _) => {
            format!("Script runtime error: {}", msg)
        }
        EvalAltResult::ErrorParsing(parse_err, _) => {
            format!("Script parse error: {}", parse_err)
        }
        EvalAltResult::ErrorVariableNotFound(var, _) => {
            format!("Variable not found: {}", var)
        }
        EvalAltResult::ErrorFunctionNotFound(func, _) => {
            format!("Function not found: {}", func)
        }
        EvalAltResult::ErrorIndexNotFound(idx, _) => {
            format!("Index not found: {}", idx)
        }
        EvalAltResult::ErrorInFunctionCall(func, _, inner, _) => {
            format!("Error in function '{}': {}", func, inner)
        }
        _ => format!("Script error: {}", err),
    }
}

/// Check if a Rhai error is due to operation limit.
///
/// # Arguments
///
/// * `err` - The Rhai error to check
///
/// # Returns
///
/// `true` if the error is an operation limit error
#[allow(dead_code)]
pub fn is_operation_limit_error(err: &EvalAltResult) -> bool {
    matches!(err, EvalAltResult::ErrorTooManyOperations(_))
}

/// Check if a Rhai error is due to termination (timeout/cancellation).
///
/// # Arguments
///
/// * `err` - The Rhai error to check
///
/// # Returns
///
/// `true` if the error is a termination error
#[allow(dead_code)]
pub fn is_termination_error(err: &EvalAltResult) -> bool {
    matches!(err, EvalAltResult::ErrorTerminated(_, _))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhai::Position;

    #[test]
    fn test_format_operation_limit_error() {
        let err = EvalAltResult::ErrorTooManyOperations(Position::NONE);
        let msg = format_rhai_error(&err);
        assert!(msg.contains("operation limit"));
    }

    #[test]
    fn test_is_operation_limit_error() {
        let limit_err = EvalAltResult::ErrorTooManyOperations(Position::NONE);
        assert!(is_operation_limit_error(&limit_err));

        let other_err = EvalAltResult::ErrorRuntime("test".into(), Position::NONE);
        assert!(!is_operation_limit_error(&other_err));
    }
}
