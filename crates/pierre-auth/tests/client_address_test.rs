// ABOUTME: Pins how the client address is found behind trusted proxies and how it is metered
// ABOUTME: Covers network parsing and containment, the X-Forwarded-For walk, the deployed chain, forged entries and IPv6 /64 keys
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![allow(missing_docs)]

use std::net::{IpAddr, Ipv4Addr};

use http::{HeaderMap, HeaderValue};
use pierre_auth::client_address::{metering_key, IpNetwork, TrustedProxies};
use pierre_auth::config::rate_limit::trusted_proxies;

/// The `X-Forwarded-For` the deployed backend received on 2026-09-26, with the
/// client replaced by a documentation address: the client, the load
/// balancer's forwarding rule, the frontend nginx's Cloud Run sandbox peer,
/// then the `0.0.0.0` Cloud Run writes for the VPC hop into the
/// internal-ingress backend.
const MEASURED_CHAIN: &str = "198.51.100.31,136.68.126.109, 169.254.169.126,0.0.0.0";

/// The backend's `TRUSTED_PROXY_CIDRS` in dev: Google's front-end ranges and
/// the load balancer's two addresses.
const DEV_TRUSTED_PROXY_CIDRS: &str =
    "35.191.0.0/16,130.211.0.0/22,136.68.126.109,2600:1901:0:3cf8::";

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
        "0.0.0.0",
        "0.255.1.2",
        "::1",
        "fd12::1",
        "fe80::1",
    ] {
        assert!(trusted.trusts(ip(internal)), "{internal}");
    }
    for public in [
        "172.32.0.1",
        "1.0.0.1",
        "8.8.8.8",
        "198.51.100.41",
        "2001:db8::1",
    ] {
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

/// carnet#623: behind the deployed chain every client read as `0.0.0.0`, the
/// last hop, so all of them shared one window.
#[test]
fn the_deployed_chain_resolves_each_client() {
    let trusted = trusted_proxies(DEV_TRUSTED_PROXY_CIDRS);
    // The backend's own TCP peer is its Cloud Run sandbox proxy.
    let peer = ip("169.254.169.126");
    let resolve = |chain: &str| trusted.client_address(peer, &forwarded_for(&[chain]));

    assert_eq!(resolve(MEASURED_CHAIN), ip("198.51.100.31"));

    // A second client through the same hops has its own address.
    let second = MEASURED_CHAIN.replace("198.51.100.31", "136.86.205.10");
    assert_eq!(resolve(&second), ip("136.86.205.10"));

    // An IPv6 client arrives through the load balancer's IPv6 address.
    assert_eq!(
        resolve("2001:db8:5::7, 2600:1901:0:3cf8::, 169.254.169.126, 0.0.0.0"),
        ip("2001:db8:5::7")
    );

    // Entries the client wrote in front of its own are never reached, even
    // ones naming a trusted hop.
    for forged in ["203.0.113.250", "0.0.0.0", "136.68.126.109", "10.0.0.1"] {
        assert_eq!(
            resolve(&format!("{forged}, {MEASURED_CHAIN}")),
            ip("198.51.100.31"),
            "{forged}"
        );
    }

    // Without the load balancer's address the walk stops at it: one key for
    // every client again, which is what TRUSTED_PROXY_CIDRS is for.
    assert_eq!(
        TrustedProxies::internal().client_address(peer, &forwarded_for(&[MEASURED_CHAIN])),
        ip("136.68.126.109")
    );
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
