use std::{
    net::Ipv4Addr,
    sync::{Arc, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};

use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::modem::Modem,
    nvs::EspDefaultNvsPartition,
    sntp::{EspSntp, SntpConf, SyncStatus},
    sys::{self, esp},
    wifi::{
        AccessPointConfiguration, AuthMethod, BlockingWifi, ClientConfiguration, Configuration,
        EspWifi,
    },
};
use goodwin_error::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use super::panel::Panel;

const RETRY_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Deserialize, Serialize)]
pub(super) struct WifiCredentials {
    pub ssid: String,
    pub password: String,
}

impl WifiCredentials {
    pub fn validate(self) -> Result<Self> {
        ensure!(
            (1..=32).contains(&self.ssid.len()) && !self.ssid.chars().any(char::is_control),
            "Invalid Wi-Fi network name"
        );
        let valid_password = match self.password.len() {
            0 => true,
            8..=63 => !self.password.chars().any(char::is_control),
            64 => self.password.bytes().all(|byte| byte.is_ascii_hexdigit()),
            _ => false,
        };
        ensure!(valid_password, "Invalid Wi-Fi password");
        Ok(self)
    }
}

#[derive(Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum WifiStatus {
    Connecting,
    Setup {
        ssid: String,
        address: Ipv4Addr,
        error: Option<&'static str>,
    },
    Connected {
        ssid: String,
        address: Ipv4Addr,
        error: Option<&'static str>,
    },
}

pub(super) struct WifiState {
    pub status: WifiStatus,
    pending: Option<(Instant, WifiCredentials)>,
}

impl WifiState {
    pub fn new() -> Self {
        Self {
            status: WifiStatus::Connecting,
            pending: None,
        }
    }

    pub fn schedule(&mut self, next: WifiCredentials) -> bool {
        if matches!(self.status, WifiStatus::Connecting) {
            return false;
        }
        // Let the HTTP server send its acknowledgement before changing IP.
        self.pending = Some((Instant::now() + Duration::from_millis(500), next));
        self.status = WifiStatus::Connecting;
        true
    }

    fn take_ready(&mut self) -> Option<WifiCredentials> {
        if self
            .pending
            .as_ref()
            .is_some_and(|(at, _)| Instant::now() >= *at)
        {
            self.pending.take().map(|(_, credentials)| credentials)
        } else {
            None
        }
    }
}

pub(super) struct Network<'d> {
    wifi: BlockingWifi<EspWifi<'d>>,
    clock: Option<EspSntp<'static>>,
    clock_synchronized: bool,
    access_point: Option<WifiCredentials>,
    setup_active: bool,
}

impl Network<'static> {
    pub(super) fn start(
        mut self,
        saved: Option<WifiCredentials>,
        panel: Arc<Panel>,
        prewarm_clock: bool,
    ) -> Result<()> {
        let status = self.restore(saved.as_ref(), None)?;
        panel.wifi.lock().expect("Wi-Fi status").status = status;
        thread::Builder::new()
            .name("goodwin-wifi".into())
            .stack_size(8 * 1024)
            .spawn(move || self.run(saved, &panel, prewarm_clock))
            .context("Start Goodwin Wi-Fi task")?;
        Ok(())
    }

    fn run(mut self, mut saved: Option<WifiCredentials>, panel: &Panel, prewarm_clock: bool) {
        let mut retry_at = Instant::now() + RETRY_INTERVAL;
        let mut clock_retry_at = Instant::now();
        let mut failures = 0;
        while !panel.reboot.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(100));
            if panel.firmware_busy() {
                continue;
            }
            let change = panel.wifi.lock().expect("Wi-Fi status").take_ready();
            if let Some(next) = change {
                failures = 0;
                match self.change(panel, &mut saved, next) {
                    Ok(status) => panel.wifi.lock().expect("Wi-Fi status").status = status,
                    Err(error) => log::warn!("Restore Wi-Fi setup: {error:#}"),
                }
                retry_at = Instant::now() + RETRY_INTERVAL;
            } else if Instant::now() >= retry_at && !self.setup_active {
                if let Err(error) = self.reconnect(panel, saved.as_ref(), &mut failures) {
                    log::warn!("Retry Wi-Fi setup: {error:#}");
                }
                retry_at = Instant::now() + RETRY_INTERVAL;
            }
            if !panel.clock_ready.load(Ordering::Acquire)
                && (prewarm_clock || panel.clock_requested.load(Ordering::Acquire))
                && panel.wifi_connected()
            {
                if Instant::now() >= clock_retry_at {
                    if let Err(error) = self.start_clock() {
                        log::warn!("Start clock synchronization: {error:#}");
                    }
                    clock_retry_at = Instant::now() + RETRY_INTERVAL;
                }
                if self.poll_clock() {
                    panel.clock_ready.store(true, Ordering::Release);
                }
            }
        }
    }

    fn change(
        &mut self,
        panel: &Panel,
        saved: &mut Option<WifiCredentials>,
        next: WifiCredentials,
    ) -> Result<WifiStatus> {
        log::info!("Applying Wi-Fi settings");
        let error = match self.join(&next) {
            Ok(ip) if panel.save_wifi(&next).is_ok() => {
                let status = connected(&next, ip, None);
                *saved = Some(next);
                return Ok(status);
            }
            Ok(_) => "save_failed",
            Err(error) => {
                log::warn!("New Wi-Fi network: {error:#}");
                "wifi_connect_failed"
            }
        };
        self.restore(saved.as_ref(), Some(error))
    }

    fn reconnect(
        &mut self,
        panel: &Panel,
        saved: Option<&WifiCredentials>,
        failures: &mut u8,
    ) -> Result<()> {
        {
            let mut state = panel.wifi.lock().expect("Wi-Fi status");
            if state.pending.is_some() {
                return Ok(());
            }
            if let Some(credentials) = saved
                && let Ok(true) = self.wifi.wifi().sta_netif().is_up()
                && let Ok(true) = self.wifi.is_connected()
                && let Ok(info) = self.wifi.wifi().sta_netif().get_ip_info()
            {
                if !matches!(state.status, WifiStatus::Connected { address, .. } if address == info.ip)
                {
                    state.status = connected(credentials, info.ip, None);
                }
                *failures = 0;
                return Ok(());
            }
            state.status = WifiStatus::Connecting;
        }
        let status = if let Some(credentials) = saved.filter(|_| *failures < 3) {
            match self.connect() {
                Ok(ip) => {
                    *failures = 0;
                    connected(credentials, ip, None)
                }
                Err(error) => {
                    log::warn!("Waiting for Wi-Fi: {error:#}");
                    *failures = failures.saturating_add(1);
                    if *failures < 3 {
                        return Ok(());
                    }
                    self.setup(Some("wifi_connect_failed"))?
                }
            }
        } else {
            self.setup(Some("wifi_connect_failed"))?
        };
        panel.wifi.lock().expect("Wi-Fi status").status = status;
        Ok(())
    }
}

impl<'d> Network<'d> {
    pub(super) fn new(modem: Modem<'d>, nvs: EspDefaultNvsPartition) -> Result<Self> {
        let events = EspSystemEventLoop::take()?;
        let wifi = BlockingWifi::wrap(EspWifi::new(modem, events.clone(), Some(nvs))?, events)?;
        // SAFETY: EspWifi has initialized the driver. Goodwin commits credentials in its own NVS namespace.
        esp!(unsafe { sys::esp_wifi_set_storage(sys::wifi_storage_t_WIFI_STORAGE_RAM) })?;
        Ok(Self {
            wifi,
            clock: None,
            clock_synchronized: false,
            access_point: None,
            setup_active: false,
        })
    }

    fn restore(
        &mut self,
        saved: Option<&WifiCredentials>,
        error: Option<&'static str>,
    ) -> Result<WifiStatus> {
        let mut reason = error;
        if let Some(credentials) = saved {
            match self.join(credentials) {
                Ok(ip) => return Ok(connected(credentials, ip, error)),
                Err(error) => {
                    log::warn!("Saved Wi-Fi network: {error:#}");
                    reason = Some("wifi_connect_failed");
                }
            }
        }
        self.setup(reason)
    }

    fn join(&mut self, credentials: &WifiCredentials) -> Result<Ipv4Addr> {
        let auth_method = if credentials.password.is_empty() {
            AuthMethod::None
        } else {
            AuthMethod::WPA2Personal
        };
        self.reconfigure(Configuration::Client(ClientConfiguration {
            ssid: credentials
                .ssid
                .as_str()
                .try_into()
                .map_err(|_| goodwin_error::report!("SSID is too long"))?,
            password: credentials
                .password
                .as_str()
                .try_into()
                .map_err(|_| goodwin_error::report!("Wi-Fi password is too long"))?,
            auth_method,
            ..Default::default()
        }))?;
        self.setup_active = false;
        self.connect()
    }

    fn setup(&mut self, error: Option<&'static str>) -> Result<WifiStatus> {
        let access_point = match &self.access_point {
            Some(access_point) => access_point,
            None => {
                let mut random = [0; 12];
                getrandom::fill(&mut random)
                    .map_err(|error| goodwin_error::report!("Wi-Fi setup random data: {error}"))?;
                let id = u32::from_be_bytes(random[..4].try_into()?);
                let password = if cfg!(feature = "esp-emulator") {
                    "goodwin-setup".to_owned()
                } else {
                    format!("{:016x}", u64::from_be_bytes(random[4..].try_into()?))
                };
                self.access_point.insert(WifiCredentials {
                    ssid: format!("Goodwin-{id:08X}"),
                    password,
                })
            }
        };
        let ssid = access_point.ssid.clone();
        let config = Configuration::AccessPoint(AccessPointConfiguration {
            ssid: ssid.as_str().try_into().expect("generated SSID"),
            password: access_point
                .password
                .as_str()
                .try_into()
                .expect("generated password"),
            auth_method: AuthMethod::WPA2Personal,
            max_connections: 2,
            ..Default::default()
        });
        self.setup_active = false;
        self.reconfigure(config)?;
        self.wifi
            .wait_netif_up()
            .context("Start Goodwin setup access point")?;
        let address = self.wifi.wifi().ap_netif().get_ip_info()?.ip;
        self.setup_active = true;
        let password = &self
            .access_point
            .as_ref()
            .expect("setup credentials")
            .password;
        log::info!("Wi-Fi setup SSID={ssid} http://{address}/settings/network");
        println!("Wi-Fi setup password for {ssid}: {password}");
        Ok(WifiStatus::Setup {
            ssid,
            address,
            error,
        })
    }

    fn reconfigure(&mut self, configuration: Configuration) -> Result<()> {
        if self.wifi.is_started()? {
            self.wifi.stop()?;
        }
        self.wifi.set_configuration(&configuration)?;
        self.wifi.start()?;
        Ok(())
    }

    fn connect(&mut self) -> Result<Ipv4Addr> {
        ensure!(!self.setup_active, "Wi-Fi setup is active");
        if !self.wifi.is_connected()? {
            self.wifi.connect().context("Connect to Wi-Fi")?;
        }
        self.wifi
            .wait_netif_up()
            .context("Wait for an IP address")?;
        let address = self.wifi.wifi().sta_netif().get_ip_info()?.ip;
        ensure!(!address.is_unspecified(), "Wi-Fi station has no IP address");
        Ok(address)
    }

    fn start_clock(&mut self) -> Result<()> {
        if self.clock.is_none() {
            let mut config = SntpConf::default();
            let servers = ["time.cloudflare.com", "time.google.com", "0.pool.ntp.org"];
            for (slot, server) in config.servers.iter_mut().zip(servers) {
                *slot = server;
            }
            self.clock = Some(EspSntp::new(&config)?);
        }
        Ok(())
    }

    fn poll_clock(&mut self) -> bool {
        if !self.clock_synchronized
            && self
                .clock
                .as_ref()
                .is_some_and(|clock| clock.get_sync_status() == SyncStatus::Completed)
        {
            self.clock_synchronized = true;
            log::info!("Clock synchronized for HTTPS");
        }
        self.clock_synchronized
    }
}

fn connected(
    credentials: &WifiCredentials,
    address: Ipv4Addr,
    error: Option<&'static str>,
) -> WifiStatus {
    log::info!("Goodwin panel http://{address}/");
    WifiStatus::Connected {
        ssid: credentials.ssid.clone(),
        address,
        error,
    }
}
