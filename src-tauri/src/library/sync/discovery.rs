use std::{
    collections::BTreeMap,
    io,
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::{hex_valid, transport::Invite};

const PROTOCOL: &str = "yap-sync-discovery/1";
const DISCOVERY_PORT: u16 = 38738;
const SCAN_WINDOW: Duration = Duration::from_millis(650);

pub(crate) fn primary_ipv4() -> Option<Ipv4Addr> {
    let route = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    // UDP connect selects the OS route without sending a packet.
    route.connect((Ipv4Addr::new(203, 0, 113, 1), 9)).ok()?;
    match route.local_addr().ok()? {
        SocketAddr::V4(address) if address.ip().is_private() => Some(*address.ip()),
        _ => None,
    }
}

#[derive(Deserialize)]
struct Reply {
    protocol: String,
    nonce: String,
    invite: String,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Peer {
    address: SocketAddr,
    name: String,
    invite: String,
}

fn verified_reply(bytes: &[u8], nonce: &str, source: SocketAddr) -> Option<Peer> {
    let reply: Reply = serde_json::from_slice(bytes).ok()?;
    if reply.protocol != PROTOCOL
        || reply.nonce != nonce
        || !hex_valid(&reply.nonce, 32)
        || reply.name.trim().is_empty()
        || reply.name.len() > 120
        || reply.name.chars().any(char::is_control)
    {
        return None;
    }
    let invite = Invite::parse(&reply.invite).ok()?;
    if !invite.modern() || invite.address.ip() != source.ip() {
        return None;
    }
    Some(Peer {
        address: invite.address,
        name: reply.name,
        invite: reply.invite,
    })
}

pub(super) fn scan() -> io::Result<Vec<Peer>> {
    let Some(ip) = primary_ipv4() else {
        return Err(io::Error::other(
            "Основной адаптер не подключён к локальной сети IPv4.",
        ));
    };
    let socket = UdpSocket::bind((ip, 0))?;
    socket.set_broadcast(true)?;
    socket.set_read_timeout(Some(Duration::from_millis(110)))?;
    let nonce = format!("{:032x}", rand::random::<u128>());
    let packet = serde_json::to_vec(&serde_json::json!({ "protocol": PROTOCOL, "nonce": nonce }))
        .map_err(io::Error::other)?;
    let destination = (Ipv4Addr::BROADCAST, DISCOVERY_PORT);
    socket.send_to(&packet, destination)?;
    let started = Instant::now();
    let mut repeated = false;
    let mut peers = BTreeMap::new();
    let mut buffer = [0; 4096];
    while started.elapsed() < SCAN_WINDOW && peers.len() < 64 {
        if !repeated && started.elapsed() >= Duration::from_millis(180) {
            repeated = true;
            socket.send_to(&packet, destination)?;
        }
        match socket.recv_from(&mut buffer) {
            Ok((length, source)) => {
                if let Some(peer) = verified_reply(&buffer[..length], &nonce, source) {
                    peers.insert(peer.address, peer);
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(peers.into_values().collect())
}
