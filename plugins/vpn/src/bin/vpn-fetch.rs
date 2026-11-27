#[cfg(target_os = "linux")]
fn main() -> goodwin_error::Result<()> {
    use std::io::Write;
    let mut args = std::env::args_os().skip(1);
    let name = args
        .next()
        .and_then(|s| s.into_string().ok())
        .unwrap_or_default();
    if name == "--subscription" {
        return goodwin_vpn::subscription::serve();
    }
    let socket = args
        .next()
        .ok_or_else(|| goodwin_error::report!("Missing VPN socket"))?;
    let result = goodwin_vpn::cache::launch(&name, std::path::Path::new(&socket));
    // On success exec replaces the helper. The worker sends the ready byte itself.
    let _ = std::io::stdout().lock().write_all(&[2]);
    result
}

#[cfg(not(target_os = "linux"))]
fn main() -> goodwin_error::Result<()> {
    goodwin_error::bail!("The VPN downloader requires Linux")
}
