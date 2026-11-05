//! Zero-configuration LAN discovery with mDNS/DNS-SD.
//!
//! A center advertises `role=center`; senders browse for it (`--to auto`).
//! Senders may advertise too, so `siren discover` can show the whole mesh.

use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;

use crate::{CoreError, Result};

pub const SERVICE_TYPE: &str = "_sirenmesh._udp.local.";

#[derive(Clone, Debug, Serialize)]
pub struct DiscoveredPeer {
    pub name: String,
    pub fullname: String,
    pub host: String,
    pub addr: String,
    pub port: u16,
    pub role: String,
    pub codec: String,
    pub id: String,
    /// Bluetooth origin bit advertised for automatic hub election.
    pub bit: Option<u8>,
    /// Bluetooth address other nodes can connect to (hub candidates).
    pub bt: Option<String>,
    /// RFCOMM channel advertised by the peer.
    pub bt_channel: Option<u8>,
    /// F6: HMAC-SHA256(psk, id|bit|bt|channel) — election credentials are
    /// only trusted when this verifies against the group key.
    pub sig: Option<String>,
    /// F16: mesh protocol version the peer advertises.
    pub proto: Option<u8>,
}

/// Keeps a service registered for as long as it is alive.
pub struct Advertiser {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Advertiser {
    pub fn start(role: &str, name: &str, port: u16, codec: &str, id: &str) -> Result<Self> {
        Self::start_full(role, name, port, codec, id, None, None, None, None)
    }

    /// Advertise the mesh node including the Bluetooth facts peers need for
    /// automatic dual-stack role election.
    pub fn start_full(
        role: &str,
        name: &str,
        port: u16,
        codec: &str,
        id: &str,
        bit: Option<u8>,
        bt: Option<&str>,
        bt_channel: Option<u8>,
        sig: Option<&str>,
    ) -> Result<Self> {
        let daemon =
            ServiceDaemon::new().map_err(|e| CoreError::Discovery(format!("mdns daemon: {e}")))?;
        let host = format!("siren-{}.local.", sanitize_hostname(&hostname()));
        let instance = format!("{name} ({role})");
        let bit_value = bit.map(|bit| bit.to_string()).unwrap_or_default();
        let bt_value = bt.unwrap_or("").to_string();
        let channel_value = bt_channel
            .map(|channel| channel.to_string())
            .unwrap_or_default();
        let proto_value = crate::protocol::MESH_PROTO_VERSION.to_string();
        let properties = [
            ("role", role),
            ("name", name),
            ("codec", codec),
            ("id", id),
            ("version", env!("CARGO_PKG_VERSION")),
            // W1: peers gate capabilities on this (BT_KEEPALIVE_PROTO) — the
            // browse side has always read it, but it was never published, so
            // the tunnel keepalive could never negotiate on real devices.
            ("proto", proto_value.as_str()),
            ("bit", bit_value.as_str()),
            ("bt", bt_value.as_str()),
            ("btch", channel_value.as_str()),
        ];
        let info = ServiceInfo::new(SERVICE_TYPE, &instance, &host, "", port, &properties[..])
            .map_err(|e| CoreError::Discovery(format!("service info: {e}")))?
            .enable_addr_auto();
        let fullname = info.get_fullname().to_string();
        daemon
            .register(info)
            .map_err(|e| CoreError::Discovery(format!("mdns register: {e}")))?;
        log::info!("advertising {fullname} on port {port}");
        Ok(Self { daemon, fullname })
    }

    pub fn fullname(&self) -> &str {
        &self.fullname
    }
}

impl Drop for Advertiser {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

/// Resolve an explicit `ip:port`/hostname, or find the first center over mDNS.
pub fn resolve_target(to: &str, auto_timeout: Duration) -> Result<SocketAddr> {
    if to.eq_ignore_ascii_case("auto") {
        let center = find_center(auto_timeout)?.ok_or_else(|| {
            CoreError::Discovery(
                "no center found; start a center or pass an explicit ip:port".into(),
            )
        })?;
        return format!("{}:{}", center.addr, center.port)
            .to_socket_addrs()
            .map_err(|e| CoreError::Discovery(format!("resolve discovered center: {e}")))?
            .next()
            .ok_or_else(|| CoreError::Discovery("discovered center has no address".into()));
    }
    let addresses: Vec<_> = to
        .to_socket_addrs()
        .map_err(|e| CoreError::Discovery(format!("cannot resolve '{to}': {e}")))?
        .collect();
    addresses
        .iter()
        .find(|addr| addr.is_ipv4())
        .or_else(|| addresses.first())
        .copied()
        .ok_or_else(|| CoreError::InvalidArgument(format!("no address for '{to}'")))
}

/// Convenience wrapper around [`Advertiser::start`].
pub fn advertise(role: &str, name: &str, port: u16, codec: &str, id: &str) -> Result<Advertiser> {
    Advertiser::start(role, name, port, codec, id)
}

#[allow(clippy::too_many_arguments)]
pub fn advertise_full(
    role: &str,
    name: &str,
    port: u16,
    codec: &str,
    id: &str,
    bit: Option<u8>,
    bt: Option<&str>,
    bt_channel: Option<u8>,
    sig: Option<&str>,
) -> Result<Advertiser> {
    Advertiser::start_full(role, name, port, codec, id, bit, bt, bt_channel, sig)
}

/// Browse for `duration` and return every resolved node.
pub fn discover(duration: Duration) -> Result<Vec<DiscoveredPeer>> {
    let peers = browse(duration, |_| false)?;
    Ok(peers)
}

/// Browse until a center appears or `timeout` expires.
pub fn find_center(timeout: Duration) -> Result<Option<DiscoveredPeer>> {
    let peers = browse(timeout, |peer| peer.role == "center")?;
    Ok(peers.into_iter().find(|peer| peer.role == "center"))
}

fn browse(
    timeout: Duration,
    stop_when: impl Fn(&DiscoveredPeer) -> bool,
) -> Result<Vec<DiscoveredPeer>> {
    let daemon =
        ServiceDaemon::new().map_err(|e| CoreError::Discovery(format!("mdns daemon: {e}")))?;
    let receiver = daemon
        .browse(SERVICE_TYPE)
        .map_err(|e| CoreError::Discovery(format!("mdns browse: {e}")))?;
    let deadline = Instant::now() + timeout;
    let mut found: HashMap<String, DiscoveredPeer> = HashMap::new();

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        match receiver.recv_timeout(wait) {
            Ok(ServiceEvent::ServiceResolved(resolved)) => {
                if let Some(peer) = peer_from_resolved(&resolved) {
                    let stop = stop_when(&peer);
                    found.insert(peer.fullname.clone(), peer);
                    if stop {
                        break;
                    }
                }
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }

    let _ = daemon.shutdown();
    let mut peers: Vec<_> = found.into_values().collect();
    peers.sort_by(|a, b| a.role.cmp(&b.role).then(a.name.cmp(&b.name)));
    Ok(peers)
}

fn peer_from_resolved(resolved: &mdns_sd::ResolvedService) -> Option<DiscoveredPeer> {
    let addresses = resolved.get_addresses_v4();
    let mut candidates: Vec<_> = addresses
        .iter()
        .copied()
        .filter(|ip| !ip.is_loopback())
        .collect();
    candidates.sort_by_key(|ip| !ip.is_private());
    let address = candidates
        .first()
        .copied()
        .or_else(|| addresses.iter().copied().next())?;

    let role = resolved
        .get_property_val_str("role")
        .unwrap_or("unknown")
        .to_string();
    let codec = resolved
        .get_property_val_str("codec")
        .unwrap_or("raw")
        .to_string();
    let id = resolved
        .get_property_val_str("id")
        .unwrap_or_default()
        .to_string();
    let fullname = resolved.get_fullname().to_string();
    let name = fullname
        .strip_suffix(&format!(".{SERVICE_TYPE}"))
        .unwrap_or(&fullname)
        .to_string();

    let bit = resolved
        .get_property_val_str("bit")
        .and_then(|value| value.parse::<u8>().ok())
        .filter(|bit| *bit < 64);
    let bt = resolved
        .get_property_val_str("bt")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_uppercase());
    let bt_channel = resolved
        .get_property_val_str("btch")
        .and_then(|value| value.parse::<u8>().ok())
        .filter(|channel| (1..=30).contains(channel));
    let sig = resolved
        .get_property_val_str("sig")
        .map(str::to_string)
        .filter(|value| !value.is_empty());
    let proto = resolved
        .get_property_val_str("proto")
        .and_then(|value| value.parse::<u8>().ok());

    Some(DiscoveredPeer {
        name,
        fullname,
        host: resolved.get_hostname().to_string(),
        addr: address.to_string(),
        port: resolved.get_port(),
        role,
        codec,
        id,
        bit,
        bt,
        bt_channel,
        sig,
        proto,
    })
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "siren-node".into())
}

fn sanitize_hostname(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "siren-node".into()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_hostnames() {
        assert_eq!(sanitize_hostname("DESKTOP-ABC_1"), "desktop-abc-1");
        assert_eq!(sanitize_hostname("---"), "siren-node");
    }
}
