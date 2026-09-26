// ABOUTME: Pins how the client address is found behind trusted proxies and how it is metered
// ABOUTME: Covers network parsing and containment, the X-Forwarded-For walk, forged entries and IPv6 /64 keys
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![allow(missing_docs)]

use std::net::{IpAddr, Ipv4Addr};

use http::{HeaderMap, HeaderValue};
use pierre_auth::client_address::{metering_key, IpNetwork, TrustedProxies};
use pierre_auth::config::rate_limit::trusted_proxies;

fn ip(raw: &str) -> IpAddr {
    raw.parse().unwrap()
}

fn forwarded_for(values: &[&str]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for value in values {
        headers.append("x-forwarded-for", HeaderValue::from_str(value).unwrap());
    }
    headers
}

#[test]
fn a_network_parses_from_cidr_or_a_bare_address() {
    let private: IpNetwork = "10.0.0.0/8".parse().unwrap();
    assert!(private.contains(ip("10.255.3.4")));
    assert!(!private.contains(ip("11.0.0.1")));
    assert_eq!(private.to_string(), "10.0.0.0/8");

    let one: IpNetwork = "34.117.9.9".parse().unwrap();
    assert_eq!(one.to_string(), "34.117.9.9/32");
    assert!(one.contains(ip("34.117.9.9")));
    assert!(!one.contains(ip("34.117.9.10")));

    let v6: IpNetwork = " 2600:1901:0:abcd::/64 ".parse().unwrap();
    assert!(v6.contains(ip("2600:1901:0:abcd:1:2:3:4")));
    assert!(!v6.contains(ip("2600:1901:0:abce::1")));
    assert!(!v6.contains(ip("10.0.0.1")), "families never match");

    let everything: IpNetwork = "0.0.0.0/0".parse().unwrap();
    assert!(everything.contains(ip("203.0.113.1")));

    for garbage in [
        "10.0.0.0/33",
        "::/129",
        "10.0.0/8",
        "nope",
        "10.0.0.0/x",
        "",
    ] {
        assert!(garbage.parse::<IpNetwork>().is_err(), "{garbage:?}");
    }
}

#[test]
fn an_ipv4_mapped_address_is_read_as_ipv4() {
    let private: IpNetwork = "10.0.0.0/8".parse().unwrap();
    assert!(private.contains(ip("::ffff:10.1.2.3")));
    assert!(TrustedProxies::internal().trusts(ip("::ffff:127.0.0.1")));
}

#[test]
fn the_internal_networks_are_trusted_and_public_ones_are_not() {
    let trusted = TrustedProxies::internal();
    for internal in [
        "127.0.0.1",
        "10.8.0.3",
        "172.31.255.1",
        "192.168.1.1",
        "169.254.169.126",
        "100.64.0.1",
        "::1",
        "fd12::1",
        "fe80::1",
    ] {
        assert!(trusted.trusts(ip(internal)), "{internal}");
    }
    for public in ["172.32.0.1", "8.8.8.8", "198.51.100.41", "2001:db8::1"] {
        assert!(!trusted.trusts(ip(public)), "{public}");
    }
}

#[test]
fn the_client_is_the_rightmost_entry_no_trusted_proxy_wrote() {
    let trusted = TrustedProxies::internal();
    let proxy = ip("10.8.0.3");

    // Client, then two internal hops appended after it
    let chain = forwarded_for(&["198.51.100.41, 169.254.1.1, 10.8.0.9"]);
    assert_eq!(trusted.client_address(proxy, &chain), ip("198.51.100.41"));

    // An entry the client wrote in front of its own is never read
    let forged = forwarded_for(&["203.0.113.250, 198.51.100.41, 169.254.1.1"]);
    assert_eq!(trusted.client_address(proxy, &forged), ip("198.51.100.41"));

    // Several header lines read as one list, in order
    let split = forwarded_for(&["203.0.113.250", "198.51.100.41", "169.254.1.1"]);
    assert_eq!(trusted.client_address(proxy, &split), ip("198.51.100.41"));

    // No header: the trusted peer is all there is
    assert_eq!(trusted.client_address(proxy, &HeaderMap::new()), proxy);

    // Every entry trusted: the leftmost of them
    let internal = forwarded_for(&["10.1.1.1, 10.2.2.2"]);
    assert_eq!(trusted.client_address(proxy, &internal), ip("10.1.1.1"));

    // An entry that does not parse stops the walk at the trusted hop right of it
    let garbled = forwarded_for(&["198.51.100.41, unknown, 169.254.1.1"]);
    assert_eq!(trusted.client_address(proxy, &garbled), ip("169.254.1.1"));
}

#[test]
fn a_peer_outside_the_trusted_networks_is_the_client() {
    let trusted = TrustedProxies::internal();
    let claims = forwarded_for(&["198.51.100.41"]);
    assert_eq!(
        trusted.client_address(ip("203.0.113.77"), &claims),
        ip("203.0.113.77")
    );
}

#[test]
fn a_public_hop_is_read_past_once_it_is_trusted() {
    // A load balancer appends its own address after the client's
    let chain = forwarded_for(&["198.51.100.41, 34.117.9.9, 169.254.1.1"]);
    let peer = ip("10.8.0.3");
    assert_eq!(
        TrustedProxies::internal().client_address(peer, &chain),
        ip("34.117.9.9"),
        "untrusted, the load balancer reads as the client"
    );
    let with_lb = trusted_proxies("34.117.9.9, not-a-network, 2600:1901::/48");
    assert_eq!(with_lb.client_address(peer, &chain), ip("198.51.100.41"));
    assert!(with_lb.trusts(ip("2600:1901:0:1::5")));
    assert!(
        with_lb.trusts(ip("10.0.0.1")),
        "the internal networks stay trusted"
    );
    assert_eq!(trusted_proxies(""), TrustedProxies::internal());
}

#[test]
fn an_ipv6_client_is_metered_by_its_slash_64() {
    assert_eq!(
        metering_key(ip("2001:db8:0:7:aaaa:bbbb:cccc:dddd")),
        ip("2001:db8:0:7::")
    );
    assert_eq!(metering_key(ip("2001:db8:0:7::1")), ip("2001:db8:0:7::"));
    assert_ne!(metering_key(ip("2001:db8:0:8::1")), ip("2001:db8:0:7::"));
    assert_eq!(
        metering_key(ip("198.51.100.41")),
        IpAddr::V4(Ipv4Addr::new(198, 51, 100, 41))
    );
    assert_eq!(
        metering_key(ip("::ffff:198.51.100.41")),
        ip("198.51.100.41")
    );
}
