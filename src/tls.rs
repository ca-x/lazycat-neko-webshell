//! Select the application's TLS provider before any outbound connections start.

pub(crate) fn initialize() {
    // Dependencies enable both ring and aws-lc-rs. Rustls cannot choose its
    // process default from features in that case, so select it before workers
    // start. Preserve a provider already installed by an earlier initializer.
    let _ = rustls23::crypto::ring::default_provider().install_default();
}

#[cfg(test)]
mod tests {
    #[test]
    fn startup_supports_tls_with_both_compiled_providers() {
        const CHILD: &str = "NEKO_TLS_PROVIDER_REGRESSION_CHILD";
        if std::env::var_os(CHILD).is_some() {
            assert!(rustls23::crypto::CryptoProvider::get_default().is_none());
            super::initialize();
            let _ = rustls23::ClientConfig::builder();
            return;
        }
        // Rustls stores its provider process-wide. A fresh process prevents a
        // different test's initialization from masking missing startup setup.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tls::tests::startup_supports_tls_with_both_compiled_providers",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
