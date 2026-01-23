//! TLS configuration loading for HTTPS support

use std::io::BufReader;

/// Load TLS configuration from certificate and key files
pub(crate) fn load_rustls_config(
    cert_path: &str,
    key_path: &str,
) -> Result<axum_server::tls_rustls::RustlsConfig, Box<dyn std::error::Error>> {
    // Install ring crypto provider (required for rustls 0.23+ with no-provider feature)
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Read certificate file
    let cert_file = std::fs::File::open(cert_path)
        .map_err(|e| format!("Failed to open certificate file '{}': {}", cert_path, e))?;
    let mut cert_reader = BufReader::new(cert_file);
    let certs: Vec<_> = rustls_pemfile::certs(&mut cert_reader)
        .filter_map(|r| r.ok())
        .collect();

    if certs.is_empty() {
        return Err(format!("No valid certificates found in '{}'", cert_path).into());
    }

    // BUG #8 fix: Read key file once into memory, then try different formats
    // This avoids reopening the file and potential handle leaks on Windows
    let key_contents = std::fs::read(key_path)
        .map_err(|e| format!("Failed to read key file '{}': {}", key_path, e))?;

    // Try PKCS8 format first
    let pkcs8_keys: Vec<_> = rustls_pemfile::pkcs8_private_keys(&mut key_contents.as_slice())
        .filter_map(|r| r.ok())
        .collect();

    let key_der: Vec<u8> = if !pkcs8_keys.is_empty() {
        // BUG #14 fix: Use if-let instead of unwrap (can't fail since we checked is_empty)
        if let Some(key) = pkcs8_keys.into_iter().next() {
            key.secret_pkcs8_der().to_vec()
        } else {
            return Err(format!("No valid PKCS8 keys in '{}'", key_path).into());
        }
    } else {
        // Try RSA key format (reuse the same bytes in memory)
        let rsa_keys: Vec<_> = rustls_pemfile::rsa_private_keys(&mut key_contents.as_slice())
            .filter_map(|r| r.ok())
            .collect();

        if rsa_keys.is_empty() {
            return Err(format!("No valid private keys found in '{}'", key_path).into());
        }
        // BUG #14 fix: Use if-let instead of unwrap
        if let Some(key) = rsa_keys.into_iter().next() {
            key.secret_pkcs1_der().to_vec()
        } else {
            return Err(format!("No valid RSA keys in '{}'", key_path).into());
        }
    };

    // Build rustls config
    let config = axum_server::tls_rustls::RustlsConfig::from_der(
        certs.into_iter().map(|c| c.to_vec()).collect(),
        key_der,
    );

    // Block on the async config creation
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(config))
        .map_err(|e| format!("Failed to create TLS config: {}", e).into())
}
