//! Process-wide rustls crypto provider.

use std::sync::Arc;

use rustls::crypto::CryptoProvider;

/// Install aws-lc-rs as the process-wide rustls `CryptoProvider`.
///
/// With rustls' `prefer-post-quantum` feature, aws-lc-rs offers the hybrid
/// post-quantum key exchange `X25519MLKEM768` first (X25519, P-256, P-384 as
/// fallbacks). Every `reqwest::Client` built afterwards inherits it.
/// Returns `Err` if a provider was already installed.
pub fn install_crypto_provider() -> Result<(), Arc<CryptoProvider>> {
    rustls::crypto::aws_lc_rs::default_provider().install_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::NamedGroup;

    #[test]
    fn installed_provider_prefers_x25519mlkem768() {
        let _ = install_crypto_provider();
        let groups: Vec<NamedGroup> = CryptoProvider::get_default()
            .expect("process-default CryptoProvider")
            .kx_groups
            .iter()
            .map(|g| g.name())
            .collect();
        eprintln!("kx_groups = {groups:?}");
        assert_eq!(groups.first(), Some(&NamedGroup::X25519MLKEM768), "kx_groups = {groups:?}");
    }

    #[test]
    fn reqwest_client_builds_on_rustls() {
        let _ = install_crypto_provider();
        reqwest::Client::builder()
            .use_rustls_tls()
            .build()
            .expect("reqwest client with rustls");
    }
}
