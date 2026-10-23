// ABOUTME: Loopback TCP relay that injects network faults between a test and a real upstream
// ABOUTME: Refuse, blackhole or delay new connections, and sever every live one, while the test runs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Fault proxy
//!
//! The failure paths that took production down were never exercised before
//! they ran there: a Postgres that refused new connections (2026-07-10) and a
//! provider that stopped answering (2026-09-21). [`FaultProxy`] sits between
//! the code under test and a real upstream on `127.0.0.1` and turns those
//! failures on and off mid-test, so a test can assert the code fails in
//! bounded time and recovers once the fault clears.
//!
//! A [`Fault`] applies to connections accepted after it is set; [`FaultProxy::sever`]
//! drops every connection already relayed, as a server restart or a failover
//! does.

use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::sleep;

/// What the proxy does with a connection it accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fault {
    /// Relay both ways untouched.
    #[default]
    Pass,
    /// Close the connection as soon as it is accepted, like a server that has
    /// no slot left or is not listening.
    Refuse,
    /// Accept the connection and never answer, like a host whose packets are
    /// dropped on the way.
    Blackhole,
    /// Relay both ways, holding every chunk for this long first.
    Latency(Duration),
}

/// A relay on `127.0.0.1` in front of `upstream`, with a switchable fault.
///
/// Dropping the proxy stops accepting and severs every live connection.
pub struct FaultProxy {
    addr: SocketAddr,
    fault: Arc<Mutex<Fault>>,
    generation: watch::Sender<u64>,
    accept: JoinHandle<()>,
}

impl FaultProxy {
    /// Start relaying to `upstream` on a free loopback port.
    ///
    /// # Errors
    ///
    /// Returns the error from binding the listening socket.
    pub async fn start(upstream: SocketAddr) -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let fault = Arc::new(Mutex::new(Fault::Pass));
        let (generation, _) = watch::channel(0_u64);
        let accept = tokio::spawn(accept_loop(
            listener,
            upstream,
            Arc::clone(&fault),
            generation.clone(),
        ));
        Ok(Self {
            addr,
            fault,
            generation,
            accept,
        })
    }

    /// The address to connect to instead of the upstream.
    #[must_use]
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Apply `fault` to every connection accepted from now on.
    pub fn set(&self, fault: Fault) {
        *self.fault.lock().unwrap_or_else(PoisonError::into_inner) = fault;
    }

    /// Drop every connection relayed so far, both sides at once.
    pub fn sever(&self) {
        self.generation.send_modify(|generation| *generation += 1);
    }
}

impl Drop for FaultProxy {
    fn drop(&mut self) {
        self.accept.abort();
        self.sever();
    }
}

async fn accept_loop(
    listener: TcpListener,
    upstream: SocketAddr,
    fault: Arc<Mutex<Fault>>,
    generation: watch::Sender<u64>,
) {
    while let Ok((client, _)) = listener.accept().await {
        let fault = *fault.lock().unwrap_or_else(PoisonError::into_inner);
        let severed = generation.subscribe();
        tokio::spawn(serve(client, upstream, fault, severed));
    }
}

/// Handles one accepted connection under the fault in force when it arrived.
async fn serve(
    client: TcpStream,
    upstream: SocketAddr,
    fault: Fault,
    mut severed: watch::Receiver<u64>,
) {
    let delay = match fault {
        Fault::Refuse => return,
        Fault::Blackhole => {
            // Hold the socket open and silent until the test severs it.
            let _ = severed.changed().await;
            drop(client);
            return;
        }
        Fault::Pass => Duration::ZERO,
        Fault::Latency(delay) => delay,
    };
    let Ok(server) = TcpStream::connect(upstream).await else {
        return;
    };
    let (client_read, client_write) = client.into_split();
    let (server_read, server_write) = server.into_split();
    tokio::select! {
        () = pump(client_read, server_write, delay) => {}
        () = pump(server_read, client_write, delay) => {}
        _ = severed.changed() => {}
    }
}

/// Copies one direction until either side closes, delaying each chunk.
async fn pump(mut from: OwnedReadHalf, mut to: OwnedWriteHalf, delay: Duration) {
    let mut buffer = vec![0_u8; 16 * 1024];
    loop {
        let read = match from.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        if !delay.is_zero() {
            sleep(delay).await;
        }
        if to.write_all(&buffer[..read]).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use tokio::io::copy;
    use tokio::time::timeout;

    use super::*;

    /// An upstream that echoes every byte back.
    async fn echo_server() -> SocketAddr {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let (mut read, mut write) = socket.split();
                    let _ = copy(&mut read, &mut write).await;
                });
            }
        });
        addr
    }

    /// Sends `ping` and waits up to `within` for it to come back.
    async fn round_trip(stream: &mut TcpStream, within: Duration) -> io::Result<Vec<u8>> {
        stream.write_all(b"ping").await?;
        let mut reply = [0_u8; 4];
        match timeout(within, stream.read_exact(&mut reply)).await {
            Ok(Ok(_)) => Ok(reply.to_vec()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "no reply")),
        }
    }

    #[tokio::test]
    async fn a_passing_proxy_relays_both_ways() {
        let proxy = FaultProxy::start(echo_server().await).await.unwrap();
        let mut stream = TcpStream::connect(proxy.addr()).await.unwrap();
        assert_eq!(
            round_trip(&mut stream, Duration::from_secs(5))
                .await
                .unwrap(),
            b"ping"
        );
    }

    #[tokio::test]
    async fn a_refusing_proxy_closes_new_connections() {
        let proxy = FaultProxy::start(echo_server().await).await.unwrap();
        proxy.set(Fault::Refuse);
        let mut stream = TcpStream::connect(proxy.addr()).await.unwrap();
        assert!(round_trip(&mut stream, Duration::from_secs(5))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_blackhole_never_answers() {
        let proxy = FaultProxy::start(echo_server().await).await.unwrap();
        proxy.set(Fault::Blackhole);
        let mut stream = TcpStream::connect(proxy.addr()).await.unwrap();
        let error = round_trip(&mut stream, Duration::from_millis(300))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[tokio::test]
    async fn latency_delays_each_chunk() {
        let proxy = FaultProxy::start(echo_server().await).await.unwrap();
        proxy.set(Fault::Latency(Duration::from_millis(150)));
        let mut stream = TcpStream::connect(proxy.addr()).await.unwrap();
        let started = Instant::now();
        assert_eq!(
            round_trip(&mut stream, Duration::from_secs(5))
                .await
                .unwrap(),
            b"ping"
        );
        assert!(
            started.elapsed() >= Duration::from_millis(300),
            "one delay each way"
        );
    }

    #[tokio::test]
    async fn sever_drops_a_live_connection_and_the_fault_applies_to_new_ones_only() {
        let proxy = FaultProxy::start(echo_server().await).await.unwrap();
        let mut live = TcpStream::connect(proxy.addr()).await.unwrap();
        assert!(round_trip(&mut live, Duration::from_secs(5)).await.is_ok());

        proxy.set(Fault::Refuse);
        assert!(
            round_trip(&mut live, Duration::from_secs(5)).await.is_ok(),
            "a fault set later leaves a relayed connection alone"
        );

        proxy.sever();
        assert!(round_trip(&mut live, Duration::from_secs(5)).await.is_err());

        proxy.set(Fault::Pass);
        let mut fresh = TcpStream::connect(proxy.addr()).await.unwrap();
        assert_eq!(
            round_trip(&mut fresh, Duration::from_secs(5))
                .await
                .unwrap(),
            b"ping"
        );
    }
}
