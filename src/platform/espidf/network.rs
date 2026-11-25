use std::{
    net::Ipv4Addr,
    time::{Duration, Instant},
};

use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::{delay::FreeRtos, modem::Modem},
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

pub(super) struct Network<'d> {
    wifi: BlockingWifi<EspWifi<'d>>,
    clock: Option<EspSntp<'static>>,
    clock_synchronized: bool,
    access_point: Option<WifiCredentials>,
    setup_active: bool,
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

    pub(super) fn join(&mut self, credentials: &WifiCredentials) -> Result<Ipv4Addr> {
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

    pub(super) fn setup(&mut self, error: Option<&'static str>) -> Result<WifiStatus> {
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
        self.reconfigure(config)?;
        self.setup_active = true;
        self.wifi
            .wait_netif_up()
            .context("Start Goodwin setup access point")?;
        let address = self.wifi.wifi().ap_netif().get_ip_info()?.ip;
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

    pub(super) fn setup_active(&self) -> bool {
        self.setup_active
    }

    fn reconfigure(&mut self, configuration: Configuration) -> Result<()> {
        if self.wifi.is_started()? {
            self.wifi.stop()?;
        }
        self.wifi.set_configuration(&configuration)?;
        self.wifi.start()?;
        Ok(())
    }

    pub(super) fn connect(&mut self) -> Result<Ipv4Addr> {
        ensure!(!self.setup_active, "Wi-Fi setup is active");
        if !self.wifi.is_connected()? {
            self.wifi.connect().context("Connect to Wi-Fi")?;
        }
        self.wifi
            .wait_netif_up()
            .context("Wait for an IP address")?;
        Ok(self.wifi.wifi().sta_netif().get_ip_info()?.ip)
    }

    pub(super) fn ready(&mut self, https: bool) -> Result<()> {
        self.connect()?;
        if https && !self.clock_synchronized {
            self.start_clock()?;
            let deadline = Instant::now() + Duration::from_secs(15);
            while !self.poll_clock() {
                ensure!(
                    Instant::now() < deadline,
                    "Wait for TLS clock synchronization"
                );
                FreeRtos::delay_ms(100);
            }
        }
        Ok(())
    }

    pub(super) fn start_clock(&mut self) -> Result<()> {
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

    pub(super) fn poll_clock(&mut self) -> bool {
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
