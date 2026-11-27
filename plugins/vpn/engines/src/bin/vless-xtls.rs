#[cfg(unix)]
#[tokio::main(flavor = "current_thread")]
async fn main() -> goodwin_vless::net::Result<()> {
    use goodwin_vless::{socks, xtls::Dialer};
    use std::sync::Arc;
    socks::serve("vless-xtls", |profile| {
        let dialer = Arc::new(Dialer::new(profile)?);
        Ok(move |header: Vec<u8>| {
            let dialer = dialer.clone();
            async move { dialer.open(&header).await }
        })
    })
    .await
}
#[cfg(not(unix))]
fn main() {
    eprintln!("The Goodwin VPN workers require Unix sockets");
    std::process::exit(1);
}
