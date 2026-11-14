//! The TCP transport: framed messages over one connection per peer, on background threads.
//!
//! - **Never on the caller's thread.** Connecting, reading and writing each run on a thread of their own; the caller
//!   (Studio's main thread) only moves bytes in and out of channels with [`Connection::send`] and
//!   [`Connection::poll`], which never block. A [`Waker`] is called whenever something arrives, so a UI that sleeps
//!   until the next event wakes for it.
//! - **Frames.** Each message is `[length: u32 LE][channel: u8][payload]`. A frame longer than the connection's
//!   [`Limits::max_frame`] closes the connection: a peer never makes the other allocate more than that at once.
//! - **Two channels on one stream.** [`Channel::Reliable`] frames are delivered in order, every one.
//!   [`Channel::Presence`] is latest-only: a new presence frame replaces one not yet written, so a slow link drops
//!   stale cursor and camera updates instead of queueing them behind the scene's edits.
//! - **Liveness.** A connection that writes nothing for [`Limits::heartbeat`] sends an empty keepalive frame; one that
//!   reads nothing for [`Limits::timeout`] closes with [`CloseReason::TimedOut`]. A peer that sends more than
//!   [`Limits::bytes_per_second`] (with a burst of a few seconds' worth) is cut off as flooding.

use std::io::{self, ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Called from a transport thread whenever an event is ready, so the owner can wake and [`Connection::poll`].
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// A waker that does nothing, for owners that poll on their own schedule (tests, servers that tick).
pub fn no_waker() -> Waker {
    Arc::new(|| {})
}

/// Which stream of a connection a frame travels on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    /// Every frame, in order: scene and text operations, joins, snapshots.
    Reliable,
    /// Latest-only: presence (cameras, selections, cursors). A newer frame replaces one not yet sent.
    Presence,
}

impl Channel {
    const fn tag(self) -> u8 {
        match self {
            Self::Reliable => 0,
            Self::Presence => 1,
        }
    }
}

/// The keepalive frame's channel tag: no payload, never delivered.
const KEEPALIVE: u8 = 2;

/// A connection's limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The longest frame read, in bytes (payload and channel byte).
    pub max_frame: usize,
    /// How long nothing may be read before the connection is closed as timed out.
    pub timeout: Duration,
    /// How long nothing may be written before a keepalive is sent.
    pub heartbeat: Duration,
    /// How long connecting may take.
    pub connect_timeout: Duration,
    /// How many bytes a second the peer may send on average; four seconds' worth may come at once.
    pub bytes_per_second: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_frame: 8 << 20,
            timeout: Duration::from_secs(12),
            heartbeat: Duration::from_secs(2),
            connect_timeout: Duration::from_secs(6),
            bytes_per_second: 16 << 20,
        }
    }
}

/// Why a connection closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseReason {
    /// This side closed it.
    Local,
    /// The peer closed it cleanly.
    PeerClosed,
    /// Nothing arrived for [`Limits::timeout`].
    TimedOut,
    /// Connecting failed: the address did not resolve, nothing listens there, or it did not answer.
    ConnectFailed(String),
    /// The peer broke the framing or a limit (a frame too long, a bad channel, flooding).
    Protocol(String),
    /// The stream failed.
    Io(String),
}

impl std::fmt::Display for CloseReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local => f.write_str("closed"),
            Self::PeerClosed => f.write_str("the other side closed the connection"),
            Self::TimedOut => f.write_str("the connection timed out"),
            Self::ConnectFailed(why) => write!(f, "could not connect: {why}"),
            Self::Protocol(why) => write!(f, "the other side broke the protocol: {why}"),
            Self::Io(why) => write!(f, "the connection failed: {why}"),
        }
    }
}

/// Something that happened on a connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The connection is up (an outgoing one has connected; an accepted one starts with this).
    Connected(SocketAddr),
    /// A frame arrived.
    Message(Channel, Vec<u8>),
    /// The connection is closed; nothing more arrives.
    Closed(CloseReason),
}

/// What the writer thread is asked to do.
enum Out {
    Frame(Vec<u8>),
    /// The presence slot has a frame.
    Presence,
    /// Flush what is queued, then close.
    Close,
}

/// One connection to a peer. Dropping it closes the connection (after what was sent is written).
pub struct Connection {
    out: Sender<Out>,
    presence: Arc<Mutex<Option<Vec<u8>>>>,
    events: Receiver<Event>,
    closed: bool,
    peer: Option<SocketAddr>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("peer", &self.peer)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

/// Splits `text` into a host and a port, `default_port` when it names none: `192.168.1.4:7350`, `[::1]:7350`,
/// `my-pc.local`, or a bare IPv6 address.
pub fn parse_address(text: &str, default_port: u16) -> Result<(String, u16), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("type the host's address, such as 192.168.1.20:7350".to_owned());
    }
    if text.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("an address has no spaces".to_owned());
    }
    let port = |p: &str| -> Result<u16, String> {
        match p.parse::<u16>() {
            Ok(0) | Err(_) => Err(format!("`{p}` is not a port (1 to 65535)")),
            Ok(port) => Ok(port),
        }
    };
    if let Some(rest) = text.strip_prefix('[') {
        let (host, after) = rest
            .split_once(']')
            .ok_or("an IPv6 address in brackets needs its `]`")?;
        return match after.strip_prefix(':') {
            Some(p) => Ok((host.to_owned(), port(p)?)),
            None if after.is_empty() => Ok((host.to_owned(), default_port)),
            None => Err("after `]` comes `:` and the port".to_owned()),
        };
    }
    match text.matches(':').count() {
        0 => Ok((text.to_owned(), default_port)),
        1 => {
            let (host, p) = text.split_once(':').ok_or("bad address")?;
            if host.is_empty() {
                return Err("the address has a port but no host".to_owned());
            }
            Ok((host.to_owned(), port(p)?))
        }
        // A bare IPv6 address.
        _ => Ok((text.to_owned(), default_port)),
    }
}

impl Connection {
    /// Starts connecting to `host:port` in the background. The first event is [`Event::Connected`] or
    /// [`Event::Closed`] with [`CloseReason::ConnectFailed`]; frames sent before then are written once connected.
    pub fn connect(host: &str, port: u16, limits: Limits, waker: Waker) -> Self {
        let (connection, parts) = Self::pair(None);
        let host = host.to_owned();
        let spawned = thread::Builder::new().name("net-connect".to_owned()).spawn(move || {
            let Parts {
                out_rx,
                presence,
                events,
                stop,
            } = parts;
            match dial(&host, port, limits.connect_timeout) {
                Ok(stream) => {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    run(
                        stream,
                        limits,
                        Parts {
                            out_rx,
                            presence,
                            events,
                            stop,
                        },
                        waker,
                    );
                }
                Err(why) => {
                    // The receiver may be gone already; nothing to tell then.
                    let _ = events.send(Event::Closed(CloseReason::ConnectFailed(why)));
                    waker();
                }
            }
        });
        let mut connection = connection;
        if let Err(error) = spawned {
            connection.closed = true;
            log::warn!("net: could not start a connection thread: {error}");
        }
        connection
    }

    /// Wraps an accepted stream.
    fn accepted(stream: TcpStream, limits: Limits, waker: Waker) -> io::Result<Self> {
        let peer = stream.peer_addr().ok();
        let (connection, parts) = Self::pair(peer);
        thread::Builder::new()
            .name("net-peer".to_owned())
            .spawn(move || run(stream, limits, parts, waker))?;
        Ok(connection)
    }

    fn pair(peer: Option<SocketAddr>) -> (Self, Parts) {
        let (out, out_rx) = mpsc::channel();
        let (events_tx, events) = mpsc::channel();
        let presence = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        (
            Self {
                out,
                presence: Arc::clone(&presence),
                events,
                closed: false,
                peer,
            },
            Parts {
                out_rx,
                presence,
                events: events_tx,
                stop,
            },
        )
    }

    /// The peer's address, once known.
    pub fn peer(&self) -> Option<SocketAddr> {
        self.peer
    }

    /// Whether the connection is closed (a [`Event::Closed`] was polled, or [`Connection::close`] was called).
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Queues `payload` on `channel`. Never blocks. Does nothing once closed.
    pub fn send(&self, channel: Channel, payload: Vec<u8>) {
        if self.closed {
            return;
        }
        match channel {
            Channel::Reliable => {
                let mut frame = Vec::with_capacity(payload.len() + 5);
                frame.extend_from_slice(&frame_len(payload.len()).to_le_bytes());
                frame.push(Channel::Reliable.tag());
                frame.extend_from_slice(&payload);
                // A closed writer drops it, as it should.
                let _ = self.out.send(Out::Frame(frame));
            }
            Channel::Presence => {
                if let Ok(mut slot) = self.presence.lock() {
                    let was_empty = slot.is_none();
                    *slot = Some(payload);
                    if was_empty {
                        let _ = self.out.send(Out::Presence);
                    }
                }
            }
        }
    }

    /// The next event, if one is ready. Never blocks.
    pub fn poll(&mut self) -> Option<Event> {
        match self.events.try_recv() {
            Ok(event) => {
                match &event {
                    Event::Connected(addr) => self.peer = Some(*addr),
                    Event::Closed(_) => self.closed = true,
                    Event::Message(..) => {}
                }
                Some(event)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                if self.closed {
                    None
                } else {
                    self.closed = true;
                    Some(Event::Closed(CloseReason::Local))
                }
            }
        }
    }

    /// Closes the connection after what is queued is written.
    pub fn close(&mut self) {
        if !self.closed {
            let _ = self.out.send(Out::Close);
        }
        self.closed = true;
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.out.send(Out::Close);
    }
}

/// A frame's length field: the payload plus its channel byte. A payload too long for a `u32` cannot be framed; it
/// is cut to the limit, and the peer closes the connection over it.
fn frame_len(payload: usize) -> u32 {
    u32::try_from(payload.saturating_add(1)).unwrap_or(u32::MAX)
}

/// The thread side of a connection.
struct Parts {
    out_rx: Receiver<Out>,
    presence: Arc<Mutex<Option<Vec<u8>>>>,
    events: Sender<Event>,
    stop: Arc<AtomicBool>,
}

/// Resolves `host` and connects to the first address that answers within `timeout`.
fn dial(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, String> {
    let addrs: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|error| format!("`{host}` is not an address that resolves ({error})"))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("`{host}` has no address"));
    }
    let mut last = String::new();
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) => {
                last = match error.kind() {
                    ErrorKind::ConnectionRefused => {
                        format!("nothing is hosting at {addr} (is the host running, and the port right?)")
                    }
                    ErrorKind::TimedOut | ErrorKind::WouldBlock => format!(
                        "{addr} did not answer in {:.1} s (a firewall, or the wrong address?)",
                        timeout.as_secs_f32()
                    ),
                    _ => format!("{addr}: {error}"),
                };
            }
        }
    }
    Err(last)
}

/// Runs a connected stream: a writer thread, and the reader on this one.
fn run(stream: TcpStream, limits: Limits, parts: Parts, waker: Waker) {
    let Parts {
        out_rx,
        presence,
        events,
        stop,
    } = parts;
    let _ = stream.set_nodelay(true);
    let peer = stream.peer_addr().ok();
    let writer_stream = match stream.try_clone() {
        Ok(writer) => writer,
        Err(error) => {
            let _ = events.send(Event::Closed(CloseReason::Io(error.to_string())));
            waker();
            return;
        }
    };
    if let Some(addr) = peer {
        let _ = events.send(Event::Connected(addr));
        waker();
    }
    let writer_stop = Arc::clone(&stop);
    let writer = thread::Builder::new()
        .name("net-write".to_owned())
        .spawn(move || write_loop(writer_stream, out_rx, presence, limits, writer_stop));
    if let Err(error) = writer {
        let _ = events.send(Event::Closed(CloseReason::Io(error.to_string())));
        waker();
        return;
    }
    let reason = read_loop(&stream, limits, &events, &waker, &stop);
    stop.store(true, Ordering::Release);
    let _ = stream.shutdown(Shutdown::Both);
    let _ = events.send(Event::Closed(reason));
    waker();
}

fn write_loop(
    mut stream: TcpStream,
    out: Receiver<Out>,
    presence: Arc<Mutex<Option<Vec<u8>>>>,
    limits: Limits,
    stop: Arc<AtomicBool>,
) {
    let keepalive = {
        let mut frame = frame_len(0).to_le_bytes().to_vec();
        frame.push(KEEPALIVE);
        frame
    };
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        let next = match out.recv_timeout(limits.heartbeat) {
            Ok(next) => next,
            Err(RecvTimeoutError::Timeout) => {
                if stream.write_all(&keepalive).is_err() {
                    break;
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let written = match next {
            Out::Frame(frame) => stream.write_all(&frame),
            Out::Presence => {
                let payload = presence.lock().ok().and_then(|mut slot| slot.take());
                match payload {
                    Some(payload) => {
                        let mut frame = Vec::with_capacity(payload.len() + 5);
                        frame.extend_from_slice(&frame_len(payload.len()).to_le_bytes());
                        frame.push(Channel::Presence.tag());
                        frame.extend_from_slice(&payload);
                        stream.write_all(&frame)
                    }
                    None => Ok(()),
                }
            }
            Out::Close => {
                let _ = stream.flush();
                break;
            }
        };
        if written.is_err() {
            break;
        }
    }
    stop.store(true, Ordering::Release);
    let _ = stream.shutdown(Shutdown::Both);
}

/// Reads frames until the stream ends, breaks a limit or times out.
fn read_loop(
    mut stream: &TcpStream,
    limits: Limits,
    events: &Sender<Event>,
    waker: &Waker,
    stop: &AtomicBool,
) -> CloseReason {
    // Short reads, so a local close is noticed quickly.
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let mut last_read = Instant::now();
    let mut budget = Budget::new(limits.bytes_per_second);
    let mut header = [0u8; 4];
    loop {
        match read_exact_watching(&mut stream, &mut header, &mut last_read, limits.timeout, stop) {
            Ok(()) => {}
            Err(reason) => return reason,
        }
        let len = u32::from_le_bytes(header) as usize;
        if len == 0 || len > limits.max_frame {
            return CloseReason::Protocol(format!("a frame of {len} bytes (at most {})", limits.max_frame));
        }
        if !budget.spend(len as u64 + 4) {
            return CloseReason::Protocol("it sent too much, too fast".to_owned());
        }
        let mut frame = vec![0u8; len];
        if let Err(reason) = read_exact_watching(&mut stream, &mut frame, &mut last_read, limits.timeout, stop) {
            return reason;
        }
        let channel = match frame[0] {
            0 => Channel::Reliable,
            1 => Channel::Presence,
            KEEPALIVE => continue,
            tag => return CloseReason::Protocol(format!("an unknown channel {tag}")),
        };
        frame.remove(0);
        if events.send(Event::Message(channel, frame)).is_err() {
            return CloseReason::Local;
        }
        waker();
    }
}

/// Fills `buf`, giving up when `stop` is set or nothing came for `timeout`.
fn read_exact_watching(
    stream: &mut &TcpStream,
    buf: &mut [u8],
    last_read: &mut Instant,
    timeout: Duration,
    stop: &AtomicBool,
) -> Result<(), CloseReason> {
    let mut filled = 0;
    while filled < buf.len() {
        if stop.load(Ordering::Acquire) {
            return Err(CloseReason::Local);
        }
        match stream.read(&mut buf[filled..]) {
            Ok(0) => return Err(CloseReason::PeerClosed),
            Ok(n) => {
                filled += n;
                *last_read = Instant::now();
            }
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if last_read.elapsed() > timeout {
                    return Err(CloseReason::TimedOut);
                }
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted | ErrorKind::BrokenPipe
                ) =>
            {
                return Err(if stop.load(Ordering::Acquire) {
                    CloseReason::Local
                } else {
                    CloseReason::PeerClosed
                });
            }
            Err(error) => return Err(CloseReason::Io(error.to_string())),
        }
    }
    Ok(())
}

/// A token bucket over bytes read: `rate` a second, four seconds' worth at most.
struct Budget {
    rate: u64,
    tokens: f64,
    at: Instant,
}

impl Budget {
    fn new(rate: u64) -> Self {
        Self {
            rate,
            tokens: (rate * 4) as f64,
            at: Instant::now(),
        }
    }

    fn spend(&mut self, bytes: u64) -> bool {
        let now = Instant::now();
        let refill = now.duration_since(self.at).as_secs_f64() * self.rate as f64;
        self.tokens = (self.tokens + refill).min((self.rate * 4) as f64);
        self.at = now;
        self.tokens -= bytes as f64;
        self.tokens >= 0.0
    }
}

/// A listening socket accepting connections on a background thread.
pub struct Listener {
    incoming: Receiver<Connection>,
    stop: Arc<AtomicBool>,
    local: SocketAddr,
    /// The accepting thread, which owns the socket: joined on drop so the port is free again once the listener is.
    thread: Option<thread::JoinHandle<()>>,
}

impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener")
            .field("local", &self.local)
            .finish_non_exhaustive()
    }
}

impl Listener {
    /// Listens on `port` of every interface (`0` picks a free port, for tests), each accepted connection with
    /// `limits`. Fails when the port is taken or not allowed.
    pub fn bind(port: u16, limits: Limits, waker: Waker) -> Result<Self, String> {
        Self::bind_addr(SocketAddr::from(([0, 0, 0, 0], port)), limits, waker)
    }

    /// Listens on `addr` (see [`Listener::bind`]).
    pub fn bind_addr(addr: SocketAddr, limits: Limits, waker: Waker) -> Result<Self, String> {
        let listener = TcpListener::bind(addr).map_err(|error| match error.kind() {
            ErrorKind::AddrInUse => format!("port {} is already in use; pick another", addr.port()),
            ErrorKind::PermissionDenied => format!("port {} is not allowed here; pick one above 1024", addr.port()),
            _ => format!("could not listen on port {}: {error}", addr.port()),
        })?;
        let local = listener.local_addr().map_err(|error| error.to_string())?;
        listener.set_nonblocking(true).map_err(|error| error.to_string())?;
        let (tx, incoming) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("net-listen".to_owned())
            .spawn(move || {
                while !thread_stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let _ = stream.set_nonblocking(false);
                            match Connection::accepted(stream, limits, Arc::clone(&waker)) {
                                Ok(connection) => {
                                    if tx.send(connection).is_err() {
                                        break;
                                    }
                                    waker();
                                }
                                Err(error) => log::warn!("net: could not start a peer's thread: {error}"),
                            }
                        }
                        Err(error) if error.kind() == ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(40));
                        }
                        Err(error) => {
                            log::debug!("net: accept failed: {error}");
                            thread::sleep(Duration::from_millis(40));
                        }
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            incoming,
            stop,
            local,
            thread: Some(thread),
        })
    }

    /// The address it listens on.
    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// The next accepted connection, if one is waiting. Never blocks.
    pub fn accept(&self) -> Option<Connection> {
        self.incoming.try_recv().ok()
    }
}

impl Drop for Listener {
    /// Stops accepting and waits for the thread (at most one 40 ms nap) so the port is released by the time the
    /// listener is gone (#639): a game served on a fixed port can start again right after Stop.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The addresses of this machine's network interfaces that others on the network could reach, best first: what the
/// host tells people to join. Found without sending anything (a UDP socket "connected" to a public address picks
/// the outgoing interface); loopback when there is no network.
pub fn local_addresses() -> Vec<std::net::IpAddr> {
    let mut found = Vec::new();
    for target in ["192.0.2.1:9", "[2001:db8::1]:9"] {
        let bind = if target.starts_with('[') { "[::]:0" } else { "0.0.0.0:0" };
        if let Ok(socket) = std::net::UdpSocket::bind(bind)
            && socket.connect(target).is_ok()
            && let Ok(addr) = socket.local_addr()
            && !addr.ip().is_unspecified()
            && !found.contains(&addr.ip())
        {
            found.push(addr.ip());
        }
    }
    if found.is_empty() {
        found.push(std::net::IpAddr::from([127, 0, 0, 1]));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Polls until `f` gives something, for at most two seconds.
    fn wait<T>(mut f: impl FnMut() -> Option<T>) -> T {
        let start = Instant::now();
        loop {
            if let Some(value) = f() {
                return value;
            }
            assert!(start.elapsed() < Duration::from_secs(5), "timed out waiting");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn message(connection: &mut Connection) -> Option<(Channel, Vec<u8>)> {
        match connection.poll()? {
            Event::Message(channel, bytes) => Some((channel, bytes)),
            Event::Connected(_) => None,
            Event::Closed(reason) => panic!("closed: {reason}"),
        }
    }

    #[test]
    fn frames_cross_both_ways_in_order() {
        let listener = Listener::bind_addr(([127, 0, 0, 1], 0).into(), Limits::default(), no_waker()).unwrap();
        let port = listener.local_addr().port();
        let mut client = Connection::connect("127.0.0.1", port, Limits::default(), no_waker());
        let mut server = wait(|| listener.accept());
        for i in 0..100u8 {
            client.send(Channel::Reliable, vec![i; usize::from(i) * 10]);
        }
        for i in 0..100u8 {
            let (channel, bytes) = wait(|| message(&mut server));
            assert_eq!(channel, Channel::Reliable);
            assert_eq!(bytes, vec![i; usize::from(i) * 10]);
        }
        server.send(Channel::Presence, b"hi".to_vec());
        assert_eq!(wait(|| message(&mut client)), (Channel::Presence, b"hi".to_vec()));
    }

    /// #639: a dropped listener has released its port, so the same port can be listened on again at once (Team
    /// Create Play's fixed port, started again right after Stop).
    #[test]
    fn a_dropped_listener_frees_its_port_at_once() {
        let mut listener = Listener::bind_addr(([127, 0, 0, 1], 0).into(), Limits::default(), no_waker()).unwrap();
        let addr = listener.local_addr();
        // Taken while it listens.
        assert!(Listener::bind_addr(addr, Limits::default(), no_waker()).is_err());
        for _ in 0..5 {
            drop(listener);
            listener = Listener::bind_addr(addr, Limits::default(), no_waker()).unwrap();
        }
    }

    #[test]
    fn a_refused_connection_says_why() {
        // Bind and drop to find a port nothing listens on.
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut client = Connection::connect("127.0.0.1", port, Limits::default(), no_waker());
        let event = wait(|| client.poll());
        assert!(
            matches!(&event, Event::Closed(CloseReason::ConnectFailed(_))),
            "{event:?}"
        );
        assert!(client.is_closed());
    }

    #[test]
    fn an_oversized_frame_closes_the_connection() {
        let limits = Limits {
            max_frame: 64,
            ..Limits::default()
        };
        let listener = Listener::bind_addr(([127, 0, 0, 1], 0).into(), limits, no_waker()).unwrap();
        let port = listener.local_addr().port();
        let client = Connection::connect("127.0.0.1", port, Limits::default(), no_waker());
        let mut server = wait(|| listener.accept());
        client.send(Channel::Reliable, vec![0; 1000]);
        let reason = wait(|| match server.poll() {
            Some(Event::Closed(reason)) => Some(reason),
            _ => None,
        });
        assert!(matches!(reason, CloseReason::Protocol(_)), "{reason:?}");
    }

    #[test]
    fn a_silent_peer_times_out() {
        let limits = Limits {
            timeout: Duration::from_millis(400),
            heartbeat: Duration::from_secs(60),
            ..Limits::default()
        };
        let raw = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = raw.local_addr().unwrap().port();
        let mut client = Connection::connect("127.0.0.1", port, limits, no_waker());
        // Accept and say nothing.
        let (_silent, _) = raw.accept().unwrap();
        let reason = wait(|| match client.poll() {
            Some(Event::Closed(reason)) => Some(reason),
            _ => None,
        });
        assert_eq!(reason, CloseReason::TimedOut);
    }

    #[test]
    fn addresses_parse_with_or_without_a_port() {
        assert_eq!(parse_address("192.168.1.4:7000", 1), Ok(("192.168.1.4".into(), 7000)));
        assert_eq!(parse_address(" my-pc ", 7350), Ok(("my-pc".into(), 7350)));
        assert_eq!(parse_address("[::1]:9", 1), Ok(("::1".into(), 9)));
        assert_eq!(parse_address("::1", 5), Ok(("::1".into(), 5)));
        assert!(parse_address("", 1).is_err());
        assert!(parse_address("host:0", 1).is_err());
        assert!(parse_address("host:99999", 1).is_err());
        assert!(parse_address(":80", 1).is_err());
        assert!(parse_address("a b:80", 1).is_err());
    }
}
