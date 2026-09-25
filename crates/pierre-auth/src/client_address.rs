// ABOUTME: The address a request came from when trusted proxies stand between the client and the server
// ABOUTME: Walks X-Forwarded-For from the right past trusted hops; an IPv6 client is metered by its /64
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Client addresses behind proxies
//!
//! Behind a proxy the server's TCP peer is the proxy, so a limit keyed on the
//! peer puts every client in one window. Each proxy that forwards a request
//! appends the address it received it from to `X-Forwarded-For`, so the
//! client is the rightmost entry that no trusted proxy wrote: walking the
//! header from the right, past every address in a trusted network, stops at
//! it. The entries to its left are whatever the client sent and are never
//! read, so a forged entry cannot choose the key. A peer outside every
//! trusted network is the client itself, and its `X-Forwarded-For` is
//! ignored outright.
//!
//! Internal networks (loopback, private, link-local, shared address space and
//! unique-local) are always trusted: a proxy in front of the server reaches it
//! from one, and no client on the public internet sends from one. A public
//! hop that appends its own address after the client's, such as a cloud load
//! balancer's forwarding rule, is added with `TRUSTED_PROXY_CIDRS`.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use http::HeaderMap;
use pierre_core::errors::AppError;
use serde::{Deserialize, Serialize};

/// The header each forwarding proxy appends its peer's address to
const FORWARDED_FOR: &str = "x-forwarded-for";

/// How many leading bits of an IPv6 client address its windows are keyed by:
/// one host is routinely handed a whole /64, and could otherwise open a fresh
/// window per address in it.
const IPV6_CLIENT_PREFIX: u8 = 64;

/// Every address sharing the first `prefix` bits of `base`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpNetwork {
    /// Any address in the network
    base: IpAddr,
    /// Leading bits every address in the network shares with `base`
    prefix: u8,
}

impl IpNetwork {
    /// `base/prefix`, or `None` when `prefix` is longer than the address.
    #[must_use]
    pub fn new(base: IpAddr, prefix: u8) -> Option<Self> {
        let base = base.to_canonical();
        (prefix <= address_bits(base)).then_some(Self { base, prefix })
    }

    /// Whether `addr` is in this network. An IPv4-mapped IPv6 address is read
    /// as the IPv4 address it maps.
    #[must_use]
    pub fn contains(&self, addr: IpAddr) -> bool {
        match (self.base, addr.to_canonical()) {
            (IpAddr::V4(base), IpAddr::V4(addr)) => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                u32::from(base) & mask == u32::from(addr) & mask
            }
            (IpAddr::V6(base), IpAddr::V6(addr)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                u128::from(base) & mask == u128::from(addr) & mask
            }
            _ => false,
        }
    }
}

impl FromStr for IpNetwork {
    type Err = AppError;

    /// `address/prefix`, or a bare address for that one address.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let invalid = || AppError::invalid_input(format!("Not an IP network: {raw}"));
        let (address, prefix) = raw.trim().split_once('/').map_or_else(
            || (raw.trim(), None),
            |(address, prefix)| (address, Some(prefix)),
        );
        let base: IpAddr = address.parse().map_err(|_| invalid())?;
        let prefix = match prefix {
            Some(bits) => bits.parse().map_err(|_| invalid())?,
            None => address_bits(base),
        };
        Self::new(base, prefix).ok_or_else(invalid)
    }
}

impl fmt::Display for IpNetwork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.base, self.prefix)
    }
}

/// The proxies the server trusts to have appended, to `X-Forwarded-For`, the
/// address they received a request from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedProxies {
    /// The internal networks, then any added to them
    networks: Vec<IpNetwork>,
}

impl TrustedProxies {
    /// The internal networks alone: loopback, RFC 1918 private, link-local,
    /// RFC 6598 shared address space, and their IPv6 counterparts.
    #[must_use]
    pub fn internal() -> Self {
        let v4 = |a, b, prefix| IpNetwork {
            base: IpAddr::V4(Ipv4Addr::new(a, b, 0, 0)),
            prefix,
        };
        let v6 = |first, prefix| IpNetwork {
            base: IpAddr::V6(Ipv6Addr::new(first, 0, 0, 0, 0, 0, 0, 0)),
            prefix,
        };
        Self {
            networks: vec![
                v4(127, 0, 8),
                v4(10, 0, 8),
                v4(172, 16, 12),
                v4(192, 168, 16),
                v4(169, 254, 16),
                v4(100, 64, 10),
                IpNetwork {
                    base: IpAddr::V6(Ipv6Addr::LOCALHOST),
                    prefix: 128,
                },
                v6(0xfc00, 7),
                v6(0xfe80, 10),
            ],
        }
    }

    /// The internal networks and `extra`.
    #[must_use]
    pub fn with(extra: impl IntoIterator<Item = IpNetwork>) -> Self {
        let mut trusted = Self::internal();
        trusted.networks.extend(extra);
        trusted
    }

    /// Whether `addr` is a trusted proxy's.
    #[must_use]
    pub fn trusts(&self, addr: IpAddr) -> bool {
        self.networks.iter().any(|network| network.contains(addr))
    }

    /// The address a request from TCP peer `peer`, carrying `headers`, came
    /// from.
    ///
    /// `peer` itself unless it is a trusted proxy's. Otherwise the rightmost
    /// `X-Forwarded-For` entry outside every trusted network; when every
    /// entry is trusted, the leftmost of them, and when an entry does not
    /// parse, the trusted one to its right. With no `X-Forwarded-For`, `peer`.
    #[must_use]
    pub fn client_address(&self, peer: IpAddr, headers: &HeaderMap) -> IpAddr {
        let mut client = peer.to_canonical();
        if !self.trusts(client) {
            return client;
        }
        let hops: Vec<&str> = headers
            .get_all(FORWARDED_FOR)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(','))
            .map(str::trim)
            .filter(|hop| !hop.is_empty())
            .collect();
        for hop in hops.into_iter().rev() {
            let Ok(addr) = hop.parse::<IpAddr>() else {
                break;
            };
            client = addr.to_canonical();
            if !self.trusts(client) {
                break;
            }
        }
        client
    }
}

impl Default for TrustedProxies {
    fn default() -> Self {
        Self::internal()
    }
}

/// The address a client's windows are keyed by: an IPv4 address itself, an
/// IPv6 address's /64.
#[must_use]
pub fn metering_key(client: IpAddr) -> IpAddr {
    match client.to_canonical() {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        IpAddr::V6(v6) => {
            let mask = u128::MAX << (128 - u32::from(IPV6_CLIENT_PREFIX));
            IpAddr::V6(Ipv6Addr::from(u128::from(v6) & mask))
        }
    }
}

/// The width of `addr` in bits.
const fn address_bits(addr: IpAddr) -> u8 {
    match addr {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    }
}
