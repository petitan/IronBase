//! Base64 automatikus felismerés és formázás
//!
//! Nagy base64 kódolt tartalmakat rövidített előnézettel helyettesítjük,
//! hogy a TUI gyorsabb és olvashatóbb legyen.

/// Minimum hossz, ami felett base64-ként kezeljük (karakter)
pub const BASE64_MIN_LENGTH: usize = 256;

/// Előnézet hossza karakterben
pub const BASE64_PREVIEW_LENGTH: usize = 16;

/// Ellenőrzi, hogy egy string valószínűleg base64 kódolt-e
///
/// A következő feltételeket vizsgálja:
/// 1. Minimum hossz (BASE64_MIN_LENGTH)
/// 2. Hossz 4 többszöröse
/// 3. Csak base64 karaktereket tartalmaz (A-Z, a-z, 0-9, +, /, =)
pub fn is_likely_base64(input: &str) -> bool {
    // 1. Minimum hossz ellenőrzés
    if input.len() < BASE64_MIN_LENGTH {
        return false;
    }

    // 2. Hossz 4 többszöröse
    if !input.len().is_multiple_of(4) {
        return false;
    }

    // 3. Csak engedélyezett karakterek: A-Z, a-z, 0-9, +, /, =
    if !input
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
    {
        return false;
    }

    true
}

/// Formázza a base64 stringet rövid előnézettel és pontos + olvasható mérettel
///
/// Kimenet: `[base64: SGVsbG8gV29ybGQh... (1,234 bytes / 1.2 KB)]`
pub fn format_base64_preview(input: &str) -> String {
    let preview: String = input.chars().take(BASE64_PREVIEW_LENGTH).collect();
    let exact_bytes = input.len();
    let human_size = format_size_human(exact_bytes);
    format!(
        "[base64: {}... ({} bytes / {})]",
        preview,
        format_bytes_with_separator(exact_bytes),
        human_size
    )
}

/// Pontos byte szám ezres elválasztóval
///
/// Példa: 1234567 -> "1,234,567"
fn format_bytes_with_separator(bytes: usize) -> String {
    let s = bytes.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

/// Olvasható méret formázás (B/KB/MB)
fn format_size_human(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Rekurzívan végigmegy a JSON értékeken és helyettesíti a base64 stringeket
pub fn sanitize_base64_values(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => {
            if is_likely_base64(s) {
                serde_json::Value::String(format_base64_preview(s))
            } else {
                value.clone()
            }
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(sanitize_base64_values).collect())
        }
        serde_json::Value::Object(obj) => serde_json::Value::Object(
            obj.iter()
                .map(|(k, v)| (k.clone(), sanitize_base64_values(v)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_likely_base64_short_string() {
        // Túl rövid
        assert!(!is_likely_base64("SGVsbG8="));
    }

    #[test]
    fn test_is_likely_base64_valid() {
        // 256+ karakter, 4 többszöröse, valid karakterek
        let long_base64 = "A".repeat(256);
        assert!(is_likely_base64(&long_base64));
    }

    #[test]
    fn test_is_likely_base64_invalid_chars() {
        // Ékezetes karakter - nem base64
        let invalid = "A".repeat(255) + "á";
        assert!(!is_likely_base64(&invalid));
    }

    #[test]
    fn test_is_likely_base64_wrong_length() {
        // 257 karakter - nem 4 többszöröse
        let wrong_len = "A".repeat(257);
        assert!(!is_likely_base64(&wrong_len));
    }

    #[test]
    fn test_format_bytes_with_separator() {
        assert_eq!(format_bytes_with_separator(123), "123");
        assert_eq!(format_bytes_with_separator(1234), "1,234");
        assert_eq!(format_bytes_with_separator(1234567), "1,234,567");
    }

    #[test]
    fn test_format_size_human() {
        assert_eq!(format_size_human(500), "500 B");
        assert_eq!(format_size_human(1024), "1.0 KB");
        assert_eq!(format_size_human(1536), "1.5 KB");
        assert_eq!(format_size_human(1048576), "1.0 MB");
    }

    #[test]
    fn test_format_base64_preview() {
        let input = "A".repeat(1024);
        let result = format_base64_preview(&input);
        assert!(result.starts_with("[base64: AAAAAAAAAAAAAAAA..."));
        assert!(result.contains("1,024 bytes"));
        assert!(result.contains("1.0 KB"));
    }

    #[test]
    fn test_sanitize_base64_values_nested() {
        let input = serde_json::json!({
            "name": "test",
            "data": "A".repeat(256),
            "nested": {
                "inner": "B".repeat(512)
            },
            "array": ["C".repeat(256), "short"]
        });

        let result = sanitize_base64_values(&input);

        // name marad, mert rövid
        assert_eq!(result["name"], "test");

        // data helyettesítve
        assert!(result["data"].as_str().unwrap().starts_with("[base64:"));

        // nested.inner helyettesítve
        assert!(result["nested"]["inner"]
            .as_str()
            .unwrap()
            .starts_with("[base64:"));

        // array[0] helyettesítve, array[1] marad
        assert!(result["array"][0].as_str().unwrap().starts_with("[base64:"));
        assert_eq!(result["array"][1], "short");
    }
}
