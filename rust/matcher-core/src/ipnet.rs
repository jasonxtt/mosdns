//! IP prefix list matcher (Go's `netlist.List`).
//!
//! All addresses are normalised to the 16-byte IPv6 space. An IPv4 address
//! `a.b.c.d` is stored as `::a.b.c.d` (zero-extended, matching Go's `to6`)
//! with `prefix_bits + 96`. The list is sorted and overlap-folded so a
//! query address can be found by binary search and prefix containment check.

use std::net::{IpAddr, Ipv6Addr};

/// A sorted, deduplicated list of IP prefixes.
///
/// Once built, call `rebuild()` to sort and fold, then `contains(addr)` for
/// read-only lookups.
pub struct IpPrefixList {
    entries: Vec<(Ipv6Addr, u8)>, // (network address, prefix bits 0-128)
    rebuilt: bool,
}

impl IpPrefixList {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            rebuilt: false,
        }
    }

    /// Append one or more prefixes. All addresses are normalised to the 16-
    /// byte space and masked. Call `rebuild()` before `contains()`.
    pub fn append(&mut self, addr: IpAddr, bits: u8) {
        let (v6, bits) = to6(addr, bits);
        let masked = mask_v6(v6, bits);
        self.entries.push((masked, bits));
        self.rebuilt = false;
    }

    /// Sort and fold overlapping/deduplicated prefixes.
    pub fn rebuild(&mut self) {
        if self.rebuilt {
            return;
        }
        self.entries.sort_by_key(|k| k.0);
        let mut out: Vec<(Ipv6Addr, u8)> = Vec::with_capacity(self.entries.len());
        for (addr, bits) in self.entries.drain(..) {
            if let Some(&(ref last_addr, last_bits)) = out.last() {
                // Same address → keep smaller prefix (more specific → fewer bits).
                if addr == *last_addr {
                    if bits < last_bits {
                        out.pop();
                        out.push((addr, bits));
                    }
                    continue;
                }
                // If this address is contained in the last prefix, skip it.
                if prefix_v6_contains(last_addr, last_bits, &addr, bits) {
                    continue;
                }
            }
            out.push((addr, bits));
        }
        self.entries = out;
        self.rebuilt = true;
    }

    /// Check `query_addr` against the sorted/folded prefix list.
    pub fn contains(&self, query_addr: IpAddr) -> bool {
        if !self.rebuilt {
            panic!("IpPrefixList must be rebuilt before contains()");
        }
        let (q, _) = to6(query_addr, 128);
        // Binary search for the last entry whose address ≤ query.
        let mut lo = 0i64;
        let mut hi = (self.entries.len() as i64) - 1;
        while lo <= hi {
            let mid = (lo + hi) / 2;
            if self.entries[mid as usize].0.cmp(&q).is_le() {
                lo = mid + 1;
            } else {
                hi = mid - 1;
            }
        }
        if hi < 0 {
            return false;
        }
        let (net_addr, net_bits) = &self.entries[hi as usize];
        prefix_contains(net_addr, *net_bits, &q)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for IpPrefixList {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert `IpAddr` to 16-byte IPv6 and adjust bits.
/// IPv4 → IPv4-mapped `::ffff:a.b.c.d` (matching Go's `netip.Addr.As16()`
/// which returns the V4-mapped form for IPv4), bits + 96.
fn to6(addr: IpAddr, bits: u8) -> (Ipv6Addr, u8) {
    match addr {
        IpAddr::V4(v4) => (v4.to_ipv6_mapped(), bits + 96),
        IpAddr::V6(v6) => (v6, bits),
    }
}

/// Mask an Ipv6Addr to the given prefix length.
fn mask_v6(addr: Ipv6Addr, bits: u8) -> Ipv6Addr {
    let bytes: [u8; 16] = addr.octets();
    let mut masked = bytes;
    let full_bytes = (bits / 8) as usize;
    masked[..full_bytes].copy_from_slice(&bytes[..full_bytes]);
    if full_bytes < 16 {
        let partial = bits % 8;
        if partial > 0 {
            let mask = 0xFF_u8 << (8 - partial);
            masked[full_bytes] = bytes[full_bytes] & mask;
            masked[(full_bytes + 1)..].fill(0);
        } else {
            masked[full_bytes..].fill(0);
        }
    }
    Ipv6Addr::from(masked)
}

/// Check if `(net, bits)` prefix contains `query_addr` (both in 16-byte space).
fn prefix_contains(net: &Ipv6Addr, bits: u8, query_addr: &Ipv6Addr) -> bool {
    if bits == 0 {
        return true; // ::0/0 matches everything
    }
    let masked_query = mask_v6(*query_addr, bits);
    *net == masked_query
}

/// Check if entry `(a2, b2)` is completely contained within prefix `(a1, b1)`.
/// Used during overlap folding.
fn prefix_v6_contains(a1: &Ipv6Addr, b1: u8, a2: &Ipv6Addr, b2: u8) -> bool {
    // If b1 > b2, a2/b2 is more specific; a1/b1 cannot contain a2/b2 if b2 > b1... actually
    // a1/b1 contains a2/b2 only if b1 <= b2 and a1 masked to b1 == a2 masked to b1.
    if b1 > b2 {
        return false; // a1/b1 is narrower (more specific), cannot contain a2/b2
    }
    let masked_a2 = mask_v6(*a2, b1);
    *a1 == masked_a2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a4(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn a6(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn v4_inside_prefix() {
        let mut l = IpPrefixList::new();
        l.append(a4("10.0.0.0"), 8);
        l.rebuild();
        assert!(l.contains(a4("10.1.2.3")));
        assert!(!l.contains(a4("11.0.0.1")));
    }

    #[test]
    fn v4_host_exact() {
        let mut l = IpPrefixList::new();
        l.append(a4("192.168.1.1"), 32);
        l.rebuild();
        assert!(l.contains(a4("192.168.1.1")));
        assert!(!l.contains(a4("192.168.1.2")));
    }

    #[test]
    fn v6_inside_prefix() {
        let mut l = IpPrefixList::new();
        l.append(a6("2001:db8::"), 32);
        l.rebuild();
        assert!(l.contains(a6("2001:db8:1::1")));
        assert!(!l.contains(a6("2001:db9::1")));
    }

    #[test]
    fn v6_host_exact() {
        let mut l = IpPrefixList::new();
        l.append(a6("2001:db8:beef::1"), 128);
        l.rebuild();
        assert!(l.contains(a6("2001:db8:beef::1")));
        assert!(!l.contains(a6("2001:db8:beef::2")));
    }

    #[test]
    fn overlap_collapse_nested() {
        let mut l = IpPrefixList::new();
        l.append(a4("192.168.0.0"), 16);
        l.append(a4("192.168.1.0"), 24);
        l.rebuild();
        assert_eq!(l.len(), 1);
        assert!(l.contains(a4("192.168.255.255")));
    }

    #[test]
    fn overlap_same_address_keeps_smaller_bits() {
        let mut l = IpPrefixList::new();
        l.append(a4("192.168.0.0"), 24);
        l.append(a4("192.168.0.0"), 16);
        l.rebuild();
        assert_eq!(l.len(), 1);
        assert!(l.contains(a4("192.168.9.9")));
    }

    #[test]
    fn disjoint_prefixes_kept() {
        let mut l = IpPrefixList::new();
        l.append(a4("10.0.0.0"), 8);
        l.append(a4("172.16.0.0"), 12);
        l.rebuild();
        assert_eq!(l.len(), 2);
    }

    #[test]
    fn v4_mapped_matches_v4_prefix() {
        let mut l = IpPrefixList::new();
        l.append(a4("1.2.3.0"), 24);
        l.rebuild();
        // IPv4-mapped IPv6 address ::ffff:1.2.3.4: in Go, this is stored as
        // ::1.2.3.0/120 (zero-extended), and ::ffff:1.2.3.4 as the query
        // address also converts to ::1.2.3.4 via to6. So they match.
        assert!(l.contains(a6("::ffff:1.2.3.4")));
        assert!(!l.contains(a6("::ffff:1.2.4.4")));
    }

    #[test]
    fn empty_never_matches() {
        let mut l = IpPrefixList::new();
        l.rebuild();
        assert!(!l.contains(a4("1.2.3.4")));
    }

    #[test]
    fn containment_boundaries() {
        let mut l = IpPrefixList::new();
        l.append(a4("192.168.0.0"), 24);
        l.append(a4("10.0.0.0"), 8);
        l.rebuild();
        assert!(l.contains(a4("192.168.0.0")));
        assert!(l.contains(a4("192.168.0.255")));
        assert!(!l.contains(a4("192.168.1.0")));
        assert!(!l.contains(a4("192.167.255.255")));
        assert!(l.contains(a4("10.0.0.0")));
        assert!(l.contains(a4("10.255.255.255")));
        assert!(!l.contains(a4("11.0.0.0")));
    }
}
