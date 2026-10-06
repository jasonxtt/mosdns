//! Listener socket creation. The IPv6 wildcard (`:port`, `[::]:port`) is
//! bound dual-stack with `IPV6_V6ONLY` cleared explicitly, so IPv4 clients
//! reach it too; every other address binds exactly as written.

use std::io;
use std::net::{Ipv4Addr, SocketAddr};

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::{TcpListener, UdpSocket};

pub(crate) async fn udp(address: SocketAddr) -> io::Result<UdpSocket> {
    match wildcard(Type::DGRAM, Protocol::UDP, address)? {
        Wildcard::Bound(socket) => UdpSocket::from_std(socket.into()),
        Wildcard::Other => UdpSocket::bind(address).await,
        Wildcard::NoIpv6(reason) => UdpSocket::bind(fallback(address, &reason)).await,
    }
}

pub(crate) async fn tcp(address: SocketAddr) -> io::Result<TcpListener> {
    match wildcard(Type::STREAM, Protocol::TCP, address)? {
        Wildcard::Bound(socket) => {
            socket.listen(1024)?;
            TcpListener::from_std(socket.into())
        }
        Wildcard::Other => TcpListener::bind(address).await,
        Wildcard::NoIpv6(reason) => TcpListener::bind(fallback(address, &reason)).await,
    }
}

/// The peer address with an IPv4-mapped IPv6 address unmapped.
pub(crate) fn canonical_peer(peer: SocketAddr) -> SocketAddr {
    SocketAddr::new(peer.ip().to_canonical(), peer.port())
}

/// The IPv4 wildcard on the same port, used when the system cannot create an
/// IPv6 socket; `reason` is reported so a different failure is not mistaken
/// for a missing IPv6 stack.
fn fallback(address: SocketAddr, reason: &io::Error) -> SocketAddr {
    crate::diag::warn_line!(
        "listener {address}: cannot create an IPv6 socket ({reason}), binding 0.0.0.0:{} instead",
        address.port()
    );
    SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), address.port())
}

enum Wildcard {
    /// A bound, non-blocking dual-stack socket.
    Bound(Socket),
    /// Not the IPv6 wildcard: bind exactly as written.
    Other,
    /// The IPv6 wildcard, but the socket could not be created.
    NoIpv6(io::Error),
}

fn wildcard(ty: Type, protocol: Protocol, address: SocketAddr) -> io::Result<Wildcard> {
    let SocketAddr::V6(v6) = address else {
        return Ok(Wildcard::Other);
    };
    if !v6.ip().is_unspecified() {
        return Ok(Wildcard::Other);
    }
    let socket = match Socket::new(Domain::IPV6, ty, Some(protocol)) {
        Ok(socket) => socket,
        Err(error) => return Ok(Wildcard::NoIpv6(error)),
    };
    socket.set_only_v6(false)?;
    if ty == Type::STREAM {
        socket.set_reuse_address(true)?;
    }
    socket.bind(&address.into())?;
    socket.set_nonblocking(true)?;
    Ok(Wildcard::Bound(socket))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ipv4_fallback_keeps_the_port() {
        let reason = io::Error::from(io::ErrorKind::Unsupported);
        assert_eq!(
            fallback("[::]:53".parse().unwrap(), &reason),
            "0.0.0.0:53".parse().unwrap()
        );
    }

    #[test]
    fn the_ipv6_wildcard_is_bound_dual_stack_and_other_addresses_are_not_touched() {
        // `NoIpv6` (a system without IPv6) is the only other acceptable outcome.
        if let Wildcard::Bound(socket) =
            wildcard(Type::DGRAM, Protocol::UDP, "[::]:0".parse().unwrap()).unwrap()
        {
            assert!(!socket.only_v6().unwrap(), "IPV6_V6ONLY must be cleared");
        }
        for address in ["127.0.0.1:0", "[::1]:0"] {
            let address: SocketAddr = address.parse().unwrap();
            assert!(matches!(
                wildcard(Type::STREAM, Protocol::TCP, address).unwrap(),
                Wildcard::Other
            ));
        }
    }

    #[test]
    fn mapped_peers_are_unmapped_and_real_ipv6_peers_are_kept() {
        assert_eq!(
            canonical_peer("[::ffff:192.0.2.7]:4000".parse().unwrap()),
            "192.0.2.7:4000".parse().unwrap()
        );
        let v6: SocketAddr = "[2001:db8::7]:4000".parse().unwrap();
        assert_eq!(canonical_peer(v6), v6);
    }
}
