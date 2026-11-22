use std::{
    collections::BTreeMap,
    net::IpAddr,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent};

const STATION_SERVICE: &str = "_yandexio._tcp.local.";
const SEARCH_TIME: Duration = Duration::from_secs(4);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Station {
    pub(crate) address: IpAddr,
    pub(crate) name: String,
}

pub(crate) fn search(report: impl Fn(Vec<Station>)) -> Result<()> {
    let daemon = ServiceDaemon::new().context("Не удалось запустить поиск в локальной сети")?;
    let result = browse(&daemon, STATION_SERVICE, SEARCH_TIME, report);
    let _ = daemon.shutdown();
    result
}

fn browse(
    daemon: &ServiceDaemon,
    service_type: &str,
    duration: Duration,
    report: impl Fn(Vec<Station>),
) -> Result<()> {
    let events = daemon
        .browse(service_type)
        .context("Не удалось начать поиск Станций")?;
    let deadline = Instant::now() + duration;
    let mut found = BTreeMap::new();
    let mut previous = Vec::new();
    while Instant::now() < deadline {
        let Ok(event) = events.recv_deadline(deadline) else {
            break;
        };
        match event {
            ServiceEvent::ServiceResolved(service) => {
                let station = station_from_records(
                    service.get_property_val_str("platform"),
                    service.get_property_val_str("name"),
                    service
                        .get_addresses()
                        .iter()
                        .map(|address| address.to_ip_addr()),
                );
                if let Some(station) = station {
                    found.insert(service.get_fullname().to_owned(), station);
                } else {
                    found.remove(service.get_fullname());
                }
            }
            ServiceEvent::ServiceRemoved(_, name) => {
                found.remove(&name);
            }
            _ => continue,
        }
        let unique: BTreeMap<_, _> = found
            .values()
            .map(|station| (station.address, station.clone()))
            .collect();
        let stations: Vec<_> = unique.into_values().collect();
        if stations != previous {
            previous.clone_from(&stations);
            report(stations);
        }
    }
    Ok(())
}

fn station_from_records(
    platform: Option<&str>,
    name: Option<&str>,
    addresses: impl IntoIterator<Item = IpAddr>,
) -> Option<Station> {
    if !platform.is_some_and(|value| value.eq_ignore_ascii_case("bergamot")) {
        return None;
    }
    let address = addresses
        .into_iter()
        .filter(|address| {
            !address.is_unspecified()
                && !address.is_loopback()
                && !address.is_multicast()
                && !matches!(address, IpAddr::V6(ip) if ip.is_unicast_link_local())
        })
        .min()?;
    let name: String = name
        .unwrap_or_default()
        .chars()
        .filter(|char| !char.is_control())
        .take(64)
        .collect();
    Some(Station {
        address,
        name: if name.trim().is_empty() {
            "Станция Мини 3"
        } else {
            name.trim()
        }
        .into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_mini_3_and_prefers_a_usable_ipv4_address() {
        let addresses = ["fe80::33", "fd00::33", "192.168.1.33", "0.0.0.0"]
            .map(|address| address.parse::<IpAddr>().unwrap());
        let station = station_from_records(Some("bergamot"), Some("Гостиная"), addresses).unwrap();
        assert_eq!(station.address, addresses[2]);
        assert_eq!(station.name, "Гостиная");
        let ipv6_station =
            station_from_records(Some("bergamot"), None, [addresses[0], addresses[1]]).unwrap();
        assert_eq!(ipv6_station.address, addresses[1]);
        assert_eq!(ipv6_station.name, "Станция Мини 3");
        assert!(station_from_records(Some("bergamot"), None, [addresses[0]]).is_none());
        assert!(station_from_records(Some("pickle"), None, addresses).is_none());
        assert!(station_from_records(None, None, addresses).is_none());
    }

    #[test]
    #[ignore = "uses local multicast sockets"]
    fn browses_for_an_advertised_station() {
        use std::{cell::Cell, net::UdpSocket};

        let socket = UdpSocket::bind("0.0.0.0:0").unwrap();
        socket.connect("224.0.0.251:5353").unwrap();
        let local = socket.local_addr().unwrap();
        drop(socket);

        let service_type = "_goodwin-test._tcp.local.";
        let publisher = ServiceDaemon::new_with_port(local.port()).unwrap();
        let client = ServiceDaemon::new_with_port(local.port()).unwrap();
        let fixture = mdns_sd::ServiceInfo::new(
            service_type,
            "Goodwin test station",
            "goodwin-test.local.",
            local.ip(),
            1961,
            &[("platform", "bergamot"), ("name", "Test Station")][..],
        )
        .unwrap();
        publisher.register(fixture).unwrap();
        let seen = Cell::new(false);
        let result = browse(&client, service_type, SEARCH_TIME, |stations| {
            if stations
                .iter()
                .any(|station| station.address == local.ip() && station.name == "Test Station")
            {
                seen.set(true);
            }
        });
        publisher.shutdown().unwrap();
        client.shutdown().unwrap();
        result.unwrap();
        assert!(seen.get());
    }
}
