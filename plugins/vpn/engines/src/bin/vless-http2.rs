#[cfg(unix)]
#[tokio::main(flavor = "current_thread")]
async fn main() -> goodwin_vless::net::Result<()> {
    use goodwin_vless::{
        http2,
        net::{Dialer, Tunnel},
        socks,
    };
    use std::sync::Arc;
    socks::serve("vless-http2", |profile| {
        let dialer = Arc::new((Dialer::new(&profile)?, profile));
        Ok(move |header: Vec<u8>| {
            let dialer = dialer.clone();
            async move {
                let (dialer, profile) = &*dialer;
                let socket = dialer.connect(profile).await?;
                let stream = http2::connect(profile, socket).await?;
                Tunnel::start(stream, &header).await
            }
        })
    })
    .await
}
#[cfg(not(unix))]
fn main() {
    eprintln!("The Goodwin VPN workers require Unix sockets");
    std::process::exit(1);
}
