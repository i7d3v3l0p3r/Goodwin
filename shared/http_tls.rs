use rustls::{
    ClientConfig, RootCertStore,
    crypto::{CryptoProvider, ring},
};

pub(crate) fn install_provider() {
    if CryptoProvider::get_default().is_none() {
        let _ = ring::default_provider().install_default();
    }
}

pub(crate) fn client_config() -> ClientConfig {
    install_provider();
    let mut config = ClientConfig::builder()
        .with_root_certificates(RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        })
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config
}
