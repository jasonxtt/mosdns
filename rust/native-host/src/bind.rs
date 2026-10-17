//! Listener socket creation. The IPv6 wildcard (`:port`, `[::]:port`) is
//! bound dual-stack with `IPV6_V6ONLY` cleared explicitly, so IPv4 clients
//! reach it too; every other address binds exactly as written.

use std::io;
use std::net::{Ipv4Addr, SocketAddr};

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::{TcpListener, UdpSocket};

/// Effective (`getsockopt`) receive buffer wanted on every UDP DNS listener.
///
/// A configuration save compiles the candidate on the single DNS thread and
/// blocks it for about 0.5-0.65 s; datagrams that arrive meanwhile wait in the
/// kernel buffer. The 5D-2b measurement showed the default (208 KiB effective,
/// about 170 small packets at roughly 1.2 KiB of kernel memory each) overflow
/// at 500 qps. 4 MiB holds about 3 400 packets: ten times that burst. Linux
/// stores twice the requested value, so 2 MiB requested (the production
/// `rmem_max`) already gives 4 MiB without privileges.
pub(crate) const UDP_RECEIVE_BUFFER_TARGET: usize = 4 * 1024 * 1024;

pub(crate) async fn udp(address: SocketAddr) -> io::Result<UdpSocket> {
    let socket = match wildcard(Type::DGRAM, Protocol::UDP, address)? {
        Wildcard::Bound(socket) => socket,
        Wildcard::Other => plain_udp(address)?,
        Wildcard::NoIpv6(reason) => plain_udp(fallback(address, &reason))?,
    };
    tune_receive_buffer(&socket, address);
    UdpSocket::from_std(socket.into())
}

/// The receive buffer the kernel actually granted (`None` when unreadable).
pub(crate) fn receive_buffer(socket: &UdpSocket) -> Option<usize> {
    socket2::SockRef::from(socket).recv_buffer_size().ok()
}

/// A bound, non-blocking UDP socket for exactly `address`.
fn plain_udp(address: SocketAddr) -> io::Result<Socket> {
    let socket = Socket::new(
        Domain::for_address(address),
        Type::DGRAM,
        Some(Protocol::UDP),
    )?;
    socket.bind(&address.into())?;
    socket.set_nonblocking(true)?;
    Ok(socket)
}

/// The socket operations the receive-buffer policy needs, so the policy can be
/// tested against a kernel with a small `rmem_max` and without privileges.
trait BufferOps {
    fn request(&mut self, bytes: usize) -> io::Result<()>;
    fn force(&mut self, bytes: usize) -> io::Result<()>;
    fn effective(&mut self) -> io::Result<usize>;
}

struct SocketBuffer<'a>(&'a Socket);

impl BufferOps for SocketBuffer<'_> {
    fn request(&mut self, bytes: usize) -> io::Result<()> {
        self.0.set_recv_buffer_size(bytes)
    }

    #[cfg(target_os = "linux")]
    fn force(&mut self, bytes: usize) -> io::Result<()> {
        rustix::net::sockopt::set_socket_recv_buffer_size_force(self.0, bytes)
            .map_err(io::Error::from)
    }

    #[cfg(not(target_os = "linux"))]
    fn force(&mut self, _bytes: usize) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn effective(&mut self) -> io::Result<usize> {
        self.0.recv_buffer_size()
    }
}

/// How the policy ended; `effective` is `None` when the kernel's value could not be read.
#[derive(Debug, PartialEq, Eq)]
struct BufferOutcome {
    effective: Option<usize>,
    /// The target was not reached; carries the reason for the warning.
    shortfall: Option<String>,
}

/// Requests half the target (Linux doubles it), then, if `rmem_max` capped it,
/// tries to force it past the cap. Nothing here can fail a listener: whatever
/// the kernel granted is kept.
fn apply_receive_buffer(ops: &mut impl BufferOps, target: usize) -> BufferOutcome {
    let mut notes = Vec::new();
    if let Err(error) = ops.request(target / 2) {
        notes.push(format!("SO_RCVBUF: {error}"));
    }
    let mut effective = ops.effective();
    if effective.as_ref().is_ok_and(|bytes| *bytes < target) {
        match ops.force(target / 2) {
            Ok(()) => effective = ops.effective(),
            Err(error) => notes.push(format!("SO_RCVBUFFORCE: {error}")),
        }
    }
    let effective = match effective {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            notes.push(format!("reading the buffer: {error}"));
            None
        }
    };
    let shortfall = (effective.is_none_or(|bytes| bytes < target)).then(|| notes.join("; "));
    BufferOutcome {
        effective,
        shortfall,
    }
}

fn tune_receive_buffer(socket: &Socket, address: SocketAddr) {
    let outcome = apply_receive_buffer(&mut SocketBuffer(socket), UDP_RECEIVE_BUFFER_TARGET);
    match outcome {
        BufferOutcome {
            effective: Some(bytes),
            shortfall: None,
        } => crate::diag::info_line!("UDP listener {address}: receive buffer {bytes} bytes"),
        BufferOutcome {
            effective,
            shortfall,
        } => crate::diag::warn_line!(
            "UDP listener {address}: receive buffer is {} bytes, wanted {UDP_RECEIVE_BUFFER_TARGET}; queries that arrive while a \
             configuration save stalls the DNS thread may be dropped (raise net.core.rmem_max) {}",
            effective.map_or_else(|| "unknown".to_owned(), |bytes| bytes.to_string()),
            shortfall.unwrap_or_default()
        ),
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

    /// A kernel with a given `rmem_max` (Linux stores twice the capped request).
    struct FakeKernel {
        max: usize,
        effective: Option<usize>,
        may_force: bool,
        fail_request: bool,
        forced: bool,
    }

    impl FakeKernel {
        fn new(max: usize, may_force: bool) -> Self {
            Self {
                max,
                effective: Some(212_992),
                may_force,
                fail_request: false,
                forced: false,
            }
        }
    }

    impl BufferOps for FakeKernel {
        fn request(&mut self, bytes: usize) -> io::Result<()> {
            if self.fail_request {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            self.effective = Some(2 * bytes.min(self.max));
            Ok(())
        }

        fn force(&mut self, bytes: usize) -> io::Result<()> {
            self.forced = true;
            if !self.may_force {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            self.effective = Some(2 * bytes);
            Ok(())
        }

        fn effective(&mut self) -> io::Result<usize> {
            self.effective
                .ok_or_else(|| io::Error::from(io::ErrorKind::Unsupported))
        }
    }

    #[test]
    fn a_large_enough_rmem_max_reaches_the_target_without_forcing() {
        let mut kernel = FakeKernel::new(2 * 1024 * 1024, false);
        let outcome = apply_receive_buffer(&mut kernel, UDP_RECEIVE_BUFFER_TARGET);
        assert_eq!(outcome.effective, Some(UDP_RECEIVE_BUFFER_TARGET));
        assert_eq!(outcome.shortfall, None);
        assert!(!kernel.forced, "SO_RCVBUFFORCE is only a fallback");
    }

    #[test]
    fn a_low_rmem_max_is_forced_past_when_the_process_may() {
        let mut kernel = FakeKernel::new(128 * 1024, true);
        let outcome = apply_receive_buffer(&mut kernel, UDP_RECEIVE_BUFFER_TARGET);
        assert_eq!(outcome.effective, Some(UDP_RECEIVE_BUFFER_TARGET));
        assert_eq!(outcome.shortfall, None);
        assert!(kernel.forced);
    }

    #[test]
    fn a_low_rmem_max_without_privilege_keeps_the_capped_value_and_only_warns() {
        let mut kernel = FakeKernel::new(128 * 1024, false);
        let outcome = apply_receive_buffer(&mut kernel, UDP_RECEIVE_BUFFER_TARGET);
        assert_eq!(outcome.effective, Some(256 * 1024));
        assert!(outcome.shortfall.unwrap().contains("SO_RCVBUFFORCE"));
    }

    #[test]
    fn failing_socket_options_never_fail_the_listener() {
        let mut kernel = FakeKernel::new(2 * 1024 * 1024, false);
        kernel.fail_request = true;
        let outcome = apply_receive_buffer(&mut kernel, UDP_RECEIVE_BUFFER_TARGET);
        // The system default stays; forcing is tried and refused.
        assert_eq!(outcome.effective, Some(212_992));
        let reason = outcome.shortfall.unwrap();
        assert!(
            reason.contains("SO_RCVBUF:") && reason.contains("SO_RCVBUFFORCE"),
            "{reason}"
        );
        let mut unreadable = FakeKernel::new(2 * 1024 * 1024, false);
        unreadable.effective = None;
        unreadable.fail_request = true;
        let outcome = apply_receive_buffer(&mut unreadable, UDP_RECEIVE_BUFFER_TARGET);
        assert_eq!(outcome.effective, None);
        assert!(outcome.shortfall.is_some());
    }

    /// The most a plain `SO_RCVBUF` request can reach on this machine.
    #[cfg(target_os = "linux")]
    fn reachable() -> usize {
        let max = std::fs::read_to_string("/proc/sys/net/core/rmem_max")
            .ok()
            .and_then(|text| text.trim().parse::<usize>().ok())
            .unwrap_or(0);
        UDP_RECEIVE_BUFFER_TARGET.min(2 * max).max(1)
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn every_udp_binding_path_enlarges_the_receive_buffer() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            // The IPv6 wildcard (dual-stack path, or the plain IPv4 fallback on a host without IPv6).
            let wildcard = udp("[::]:0".parse().unwrap()).await.unwrap();
            // Any other address (the plain path).
            let loopback = udp("127.0.0.1:0".parse().unwrap()).await.unwrap();
            for socket in [&wildcard, &loopback] {
                assert!(receive_buffer(socket).unwrap() >= reachable());
            }
            // The fallback path itself: the plain creation the NoIpv6 case uses.
            let fallback = plain_udp("0.0.0.0:0".parse().unwrap()).unwrap();
            tune_receive_buffer(&fallback, "0.0.0.0:0".parse().unwrap());
            assert!(fallback.recv_buffer_size().unwrap() >= reachable());
            // Unchanged behaviour: a second bind of a bound UDP address is still refused.
            let taken = loopback.local_addr().unwrap();
            assert_eq!(
                udp(taken).await.unwrap_err().kind(),
                io::ErrorKind::AddrInUse
            );
        });
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
