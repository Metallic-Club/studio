//! Team Create: several editors on one project, one of them hosting (ADR 0011).
//!
//! The pieces stay apart so a cloud-hosted session (Online Mode) can reuse all but the transport:
//!
//! | Concern | Here |
//! |---|---|
//! | messages | [`protocol`]: what travels, read with limits |
//! | the session's authority | [`host::Host`]: who joins, under what name, what is forwarded to whom |
//! | a participant | [`client::Client`]: the join, the project as it arrives, operations held until it is open |
//! | transport and hosting | [`HostServer`] and [`ClientLink`]: the two above over TCP ([`crate::transport`]) |
//! | discovery | [`probe`]: asks an address what it hosts, for the projects page |
//!
//! Neither [`host::Host`] nor [`client::Client`] knows it runs over TCP, or that the host is a person's Studio:
//! a relay would feed them the same events.

pub mod client;
pub mod host;
pub mod presence;
pub mod protocol;
pub mod scene;
pub mod value;

#[cfg(test)]
mod scene_tests;

use std::collections::BTreeMap;
use std::time::Duration;

use client::Client;
use host::{Host, PeerId};
use protocol::{ClientMsg, HostMsg, PROTOCOL_VERSION, Purpose, SessionInfo};

use crate::transport::{Channel, Connection, Event, Limits, Listener, Waker};

/// The transport limits a host reads participants with: frames up to 8 MiB (a large paste), a flood cut off.
pub fn host_limits() -> Limits {
    Limits::default()
}

/// The transport limits a participant reads the host with: frames up to a chunk and a large message.
pub fn client_limits() -> Limits {
    Limits {
        max_frame: 16 << 20,
        // The project arrives at the start; let it come fast.
        bytes_per_second: 256 << 20,
        ..Limits::default()
    }
}

/// A [`Host`] over TCP: listens on a port and moves each connection's frames in and out of the session.
#[derive(Debug)]
pub struct HostServer {
    listener: Listener,
    connections: BTreeMap<PeerId, Connection>,
    next_peer: u64,
    /// The session.
    pub host: Host,
}

impl HostServer {
    /// Starts listening on `port` (every interface) for `host`'s session.
    pub fn start(port: u16, host: Host, waker: Waker) -> Result<Self, String> {
        let listener = Listener::bind(port, host_limits(), waker)?;
        Ok(Self::with_listener(listener, host))
    }

    /// `host`'s session on `listener`.
    pub fn with_listener(listener: Listener, host: Host) -> Self {
        Self {
            listener,
            connections: BTreeMap::new(),
            next_peer: 1,
            host,
        }
    }

    /// The port it listens on.
    pub fn port(&self) -> u16 {
        self.listener.local_addr().port()
    }

    /// How many connections are open.
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Accepts new connections and feeds what arrived to the session. Never blocks.
    pub fn receive(&mut self) {
        while let Some(connection) = self.listener.accept() {
            let peer = PeerId(self.next_peer);
            self.next_peer += 1;
            self.connections.insert(peer, connection);
            self.host.connected(peer);
        }
        let mut closed = Vec::new();
        for (&peer, connection) in &mut self.connections {
            // At most a frame's worth of work per connection per call is not needed: the session is cheap per
            // message, and draining keeps latency low.
            while let Some(event) = connection.poll() {
                match event {
                    Event::Connected(_) => {}
                    Event::Message(channel, bytes) => self.host.received(peer, channel, &bytes),
                    Event::Closed(reason) => {
                        closed.push((peer, reason.to_string()));
                        break;
                    }
                }
            }
        }
        for (peer, reason) in closed {
            self.connections.remove(&peer);
            self.host.disconnected(peer, &reason);
        }
    }

    /// Sends what the session queued and closes the connections it let go. Never blocks.
    pub fn flush(&mut self) {
        for (peer, channel, bytes) in self.host.take_outbox() {
            if let Some(connection) = self.connections.get(&peer) {
                connection.send(channel, bytes);
            }
        }
        for peer in self.host.take_closing() {
            if let Some(mut connection) = self.connections.remove(&peer) {
                connection.close();
            }
        }
    }
}

/// A [`Client`] over TCP.
#[derive(Debug)]
pub struct ClientLink {
    connection: Connection,
    /// The session.
    pub client: Client,
}

impl ClientLink {
    /// Connects to `host:port` in the background and joins with `client`'s hello.
    pub fn connect(host: &str, port: u16, client: Client, waker: Waker) -> Self {
        let connection = Connection::connect(host, port, client_limits(), waker);
        let mut link = Self { connection, client };
        link.flush();
        link
    }

    /// Feeds what arrived to the session. Never blocks.
    pub fn receive(&mut self) {
        while let Some(event) = self.connection.poll() {
            match event {
                Event::Connected(_) => {}
                Event::Message(channel, bytes) => self.client.received(channel, &bytes),
                Event::Closed(reason) => {
                    self.client.disconnected(&reason.to_string());
                    break;
                }
            }
        }
    }

    /// Sends what the session queued. Never blocks.
    pub fn flush(&mut self) {
        for (channel, bytes) in self.client.take_outbox() {
            self.connection.send(channel, bytes);
        }
    }

    /// Leaves the session and closes the connection after the goodbye.
    pub fn leave(&mut self) {
        self.client.leave();
        self.flush();
        self.connection.close();
    }

    /// Whether the connection is closed.
    pub fn is_closed(&self) -> bool {
        self.connection.is_closed()
    }
}

/// What a probe found at an address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probed {
    /// The session hosted there.
    pub session: SessionInfo,
    /// How many users are in it.
    pub users: u32,
}

/// Asks `host:port` what it hosts, waiting at most `timeout`. Blocks: call it on a worker thread. An error says why
/// nothing answered in a way the user can act on.
pub fn probe(host: &str, port: u16, engine: &str, fingerprint: u64, timeout: Duration) -> Result<Probed, String> {
    let limits = Limits {
        connect_timeout: timeout,
        timeout,
        ..client_limits()
    };
    let mut connection = Connection::connect(host, port, limits, crate::transport::no_waker());
    connection.send(
        Channel::Reliable,
        ClientMsg::Hello {
            protocol: PROTOCOL_VERSION,
            engine: engine.to_owned(),
            fingerprint,
            purpose: Purpose::Probe,
        }
        .encode(),
    );
    let deadline = std::time::Instant::now() + timeout * 2;
    while std::time::Instant::now() < deadline {
        match connection.poll() {
            Some(Event::Message(_, bytes)) => {
                return match HostMsg::decode(&bytes) {
                    Ok(HostMsg::ProbeInfo { session, users }) => Ok(Probed { session, users }),
                    Ok(HostMsg::Refused(reason)) => Err(reason),
                    Ok(_) => Err("it answered something other than Team Create".to_owned()),
                    Err(error) => Err(format!("it answered in a way Studio does not read ({error})")),
                };
            }
            Some(Event::Closed(reason)) => return Err(reason.to_string()),
            Some(Event::Connected(_)) => {}
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    Err("it did not answer in time".to_owned())
}

#[cfg(test)]
mod tests;
