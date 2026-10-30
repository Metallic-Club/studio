//! QUIC transport (quinn, TLS 1.3 with a pinned self-signed certificate, ALPN [`ALPN`]).
//!
//! quinn runs on a tokio runtime in a background thread: one per [`QuicServer`], one shared by every client
//! connection of the process. The game thread talks to it through queues, so [`Connection`] stays non-blocking.
//!
//! Streams: the client opens Control (bidirectional) and Remote; the server opens World and one stream per Bulk
//! payload. The opener writes the channel's tag byte first; then each frame is a LEB128 length and the bytes. The
//! server allows a client [`CLIENT_MAX_STREAMS`] open streams (one bidirectional), so QUIC flow control refuses more.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{
    AsyncUdpSocket, ConnectionError, Endpoint, EndpointConfig, IdleTimeout, ReadExactError,
    RecvStream, Runtime, SendDatagramError, SendStream, TokioRuntime, TransportConfig, VarInt,
};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};

use super::cert::{self, CERT_NAME, PinnedVerifier, ServerIdentity};
use super::conditioner::ConditionedSocket;
use super::limits::{
    ALPN, CLIENT_MAX_STREAMS, DatagramLimiter, HELLO_TIMEOUT, IDLE_TIMEOUT, KEEP_ALIVE,
    MAX_DATAGRAM,
};
use super::{
    Channel, CloseReason, Connection, Incoming, LinkConditioner, LinkStats, Listener, Side,
};
use crate::NetError;

/// Events (frames, close) waiting for `poll`; when full, stream reads pause and QUIC flow control pushes back.
const EVENT_QUEUE: usize = 256;
/// Server-to-client unidirectional streams a client accepts at once (World plus Bulk payloads).
const CLIENT_INCOMING_UNI: u32 = 32;
/// Starting UDP payload size; leaves room for a [`MAX_DATAGRAM`] payload plus QUIC overhead.
const INITIAL_MTU: u16 = 1280;
/// Close reasons are cut to this many bytes.
const MAX_REASON: usize = 200;

/// A QUIC server: binds a UDP socket, makes a fresh certificate and accepts clients.
pub struct QuicServer {
    local_addr: SocketAddr,
    cert_sha256: [u8; 32],
    ready: mpsc::UnboundedReceiver<QuicConnection>,
    runtime: Arc<RuntimeThread>,
}

impl QuicServer {
    /// Binds `addr` (port 0 picks a free port) with a fresh ECDSA P-256 certificate valid for 13 days. With a
    /// conditioner, the server's socket delays and drops packets both ways.
    pub fn bind(
        addr: SocketAddr,
        conditioner: Option<LinkConditioner>,
    ) -> Result<QuicServer, NetError> {
        let identity = cert::self_signed()?;
        let cert_sha256 = identity.sha256;
        let server_config = server_config(identity)?;
        let socket = UdpSocket::bind(addr)?;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let handle = rt.handle().clone();
        let endpoint = {
            let _context = rt.enter();
            let socket = TokioRuntime.wrap_udp_socket(socket)?;
            let socket: Arc<dyn AsyncUdpSocket> = match conditioner {
                Some(cond) => Arc::new(ConditionedSocket::new(socket, cond)),
                None => socket,
            };
            Endpoint::new_with_abstract_socket(
                EndpointConfig::default(),
                Some(server_config),
                socket,
                Arc::new(TokioRuntime),
            )?
        };
        let local_addr = endpoint.local_addr()?;
        let (ready_tx, ready) = mpsc::unbounded_channel();
        handle.spawn(accept_loop(endpoint.clone(), ready_tx));
        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        std::thread::Builder::new()
            .name("metallic-net-server".into())
            .spawn(move || {
                rt.block_on(async move {
                    let _ = stop_rx.await;
                    endpoint.close(VarInt::from_u32(0), b"server stopped");
                    let _ = tokio::time::timeout(Duration::from_millis(500), endpoint.wait_idle())
                        .await;
                });
            })?;
        Ok(QuicServer {
            local_addr,
            cert_sha256,
            ready,
            runtime: Arc::new(RuntimeThread {
                stop: Mutex::new(Some(stop_tx)),
            }),
        })
    }

    /// The bound address (with the real port when bound to port 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// SHA-256 of the certificate (DER), which clients pin.
    pub fn cert_sha256(&self) -> [u8; 32] {
        self.cert_sha256
    }
}

impl Listener for QuicServer {
    fn accept(&mut self) -> Option<Box<dyn Connection>> {
        let mut conn = self.ready.try_recv().ok()?;
        conn.runtime = Some(self.runtime.clone());
        Some(Box::new(conn))
    }
}

/// Connects to QUIC servers.
pub struct QuicClient;

impl QuicClient {
    /// Connects to `addr`, accepting only the certificate whose SHA-256 is `pin`; blocks for at most `timeout`.
    /// Opens the Control and Remote streams; send the Hello on Control within [`HELLO_TIMEOUT`].
    pub fn connect(
        addr: SocketAddr,
        pin: [u8; 32],
        timeout: Duration,
    ) -> Result<Box<dyn Connection>, NetError> {
        let config = Self::quinn_config(pin)?;
        let handle = client_runtime()?;
        let (tx, rx) = std::sync::mpsc::channel();
        handle.spawn(async move {
            let result = match tokio::time::timeout(timeout, connect_task(addr, config)).await {
                Ok(result) => result,
                Err(_) => Err(NetError::Timeout),
            };
            let _ = tx.send(result);
        });
        match rx.recv_timeout(timeout + Duration::from_secs(1)) {
            Ok(result) => result.map(|conn| Box::new(conn) as Box<dyn Connection>),
            Err(_) => Err(NetError::Timeout),
        }
    }

    /// The quinn client configuration [`QuicClient::connect`] uses (pinned verifier, ALPN, limits), for tools and
    /// tests that drive QUIC directly. The server name to connect with is `"metallic"`.
    pub fn quinn_config(pin: [u8; 32]) -> Result<quinn::ClientConfig, NetError> {
        let provider = cert::provider();
        let mut tls = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| NetError::Tls(e.to_string()))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedVerifier::new(pin, provider)))
            .with_no_client_auth();
        tls.alpn_protocols = vec![ALPN.to_vec()];
        let crypto = QuicClientConfig::try_from(tls).map_err(|e| NetError::Tls(e.to_string()))?;
        let mut config = quinn::ClientConfig::new(Arc::new(crypto));
        config.transport_config(Arc::new(transport_config(Side::Client)?));
        Ok(config)
    }
}

fn server_config(identity: ServerIdentity) -> Result<quinn::ServerConfig, NetError> {
    let mut tls = rustls::ServerConfig::builder_with_provider(cert::provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| NetError::Tls(e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(vec![identity.cert], identity.key)
        .map_err(|e| NetError::Tls(e.to_string()))?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let crypto = QuicServerConfig::try_from(tls).map_err(|e| NetError::Tls(e.to_string()))?;
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    config.transport_config(Arc::new(transport_config(Side::Server)?));
    config.migration(true);
    Ok(config)
}

/// Limits on what the peer may open, plus timeouts. A server's config limits its clients.
fn transport_config(side: Side) -> Result<TransportConfig, NetError> {
    let mut t = TransportConfig::default();
    t.max_idle_timeout(Some(
        IdleTimeout::try_from(IDLE_TIMEOUT).map_err(|e| NetError::Tls(e.to_string()))?,
    ));
    t.keep_alive_interval(Some(KEEP_ALIVE));
    t.initial_mtu(INITIAL_MTU);
    match side {
        Side::Server => {
            t.max_concurrent_bidi_streams(VarInt::from_u32(1));
            t.max_concurrent_uni_streams(VarInt::from_u32(CLIENT_MAX_STREAMS - 1));
        }
        Side::Client => {
            t.max_concurrent_bidi_streams(VarInt::from_u32(0));
            t.max_concurrent_uni_streams(VarInt::from_u32(CLIENT_INCOMING_UNI));
        }
    }
    Ok(t)
}

/// Keeps a server's runtime thread running while the server or any of its connections exists.
struct RuntimeThread {
    stop: Mutex<Option<oneshot::Sender<()>>>,
}

impl Drop for RuntimeThread {
    fn drop(&mut self) {
        if let Some(stop) = self
            .stop
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            let _ = stop.send(());
        }
    }
}

/// The runtime every client connection of this process shares; it runs until the process exits.
fn client_runtime() -> Result<&'static Handle, NetError> {
    static RUNTIME: OnceLock<Result<Handle, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?;
            let handle = rt.handle().clone();
            std::thread::Builder::new()
                .name("metallic-net-client".into())
                .spawn(move || rt.block_on(std::future::pending::<()>()))
                .map_err(|e| e.to_string())?;
            Ok(handle)
        })
        .as_ref()
        .map_err(|e| NetError::Io(e.clone()))
}

async fn connect_task(
    addr: SocketAddr,
    config: quinn::ClientConfig,
) -> Result<QuicConnection, NetError> {
    let local_ip = match (addr.is_ipv4(), addr.ip().is_loopback()) {
        (true, true) => IpAddr::V4(Ipv4Addr::LOCALHOST),
        (true, false) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        (false, true) => IpAddr::V6(Ipv6Addr::LOCALHOST),
        (false, false) => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
    };
    let socket = UdpSocket::bind(SocketAddr::new(local_ip, 0))?;
    let endpoint = Endpoint::new(
        EndpointConfig::default(),
        None,
        socket,
        Arc::new(TokioRuntime),
    )?;
    let failed = |e: &dyn std::fmt::Display| NetError::Connect(e.to_string());
    let conn = endpoint
        .connect_with(config, addr, CERT_NAME)
        .map_err(|e| failed(&e))?
        .await
        .map_err(|e| failed(&e))?;
    let (mut control, control_recv) = conn.open_bi().await.map_err(|e| failed(&e))?;
    control
        .write_all(&[Channel::Control.tag()])
        .await
        .map_err(|e| failed(&e))?;
    let mut remote = conn.open_uni().await.map_err(|e| failed(&e))?;
    remote
        .write_all(&[Channel::Remote.tag()])
        .await
        .map_err(|e| failed(&e))?;
    let streams = Streams {
        control: ControlSlot::Ready(control),
        control_recv: Some(control_recv),
        long: Some(remote),
    };
    Ok(QuicConnection::start(
        conn,
        Side::Client,
        streams,
        Some(endpoint),
    ))
}

async fn accept_loop(endpoint: Endpoint, ready: mpsc::UnboundedSender<QuicConnection>) {
    while let Some(incoming) = endpoint.accept().await {
        let ready = ready.clone();
        tokio::spawn(async move {
            let Ok(connecting) = incoming.accept() else {
                return;
            };
            let Ok(Ok(conn)) = tokio::time::timeout(IDLE_TIMEOUT, connecting).await else {
                return;
            };
            let world = async {
                let mut world = conn.open_uni().await.ok()?;
                world.write_all(&[Channel::World.tag()]).await.ok()?;
                Some(world)
            };
            let Ok(Some(world)) = tokio::time::timeout(HELLO_TIMEOUT, world).await else {
                conn.close(VarInt::from_u32(4), b"could not open the world stream");
                return;
            };
            let (control_tx, control_rx) = oneshot::channel();
            let streams = Streams {
                control: ControlSlot::Waiting(control_rx),
                control_recv: None,
                long: Some(world),
            };
            let qc = QuicConnection::start(conn.clone(), Side::Server, streams, None);
            tokio::spawn(accept_control(
                conn.clone(),
                qc.shared.clone(),
                qc.events_tx.clone(),
                control_tx,
            ));
            let _ = ready.send(qc);
        });
    }
}

/// State the background tasks share with the game-side handle.
#[derive(Default)]
struct Shared {
    /// Why this side closed, for the close event.
    local_close: Mutex<Option<CloseReason>>,
    datagrams_received: AtomicU64,
    datagrams_dropped: AtomicU64,
    hello_seen: AtomicBool,
}

/// Closes the connection, remembering why for this side's own `Closed` event.
fn close_with(conn: &quinn::Connection, shared: &Shared, reason: CloseReason) {
    let (code, text) = reason.to_wire();
    let mut cut = text.len().min(MAX_REASON);
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let text = text[..cut].to_string();
    shared
        .local_close
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert(reason);
    conn.close(VarInt::from_u32(code), text.as_bytes());
}

fn protocol_error(conn: &quinn::Connection, shared: &Shared, message: impl Into<String>) {
    close_with(conn, shared, CloseReason::ProtocolError(message.into()));
}

enum ControlSlot {
    Ready(SendStream),
    Waiting(oneshot::Receiver<SendStream>),
    Gone,
}

struct Streams {
    control: ControlSlot,
    control_recv: Option<RecvStream>,
    /// World (server) or Remote (client).
    long: Option<SendStream>,
}

/// The game-side handle of a QUIC connection.
struct QuicConnection {
    conn: quinn::Connection,
    side: Side,
    events: mpsc::Receiver<Incoming>,
    events_tx: mpsc::Sender<Incoming>,
    sends: mpsc::UnboundedSender<(Channel, Vec<u8>)>,
    shared: Arc<Shared>,
    closed: bool,
    _endpoint: Option<Endpoint>,
    runtime: Option<Arc<RuntimeThread>>,
}

impl QuicConnection {
    /// Starts the background tasks. Must run inside the runtime.
    fn start(
        conn: quinn::Connection,
        side: Side,
        streams: Streams,
        endpoint: Option<Endpoint>,
    ) -> QuicConnection {
        let (events_tx, events) = mpsc::channel(EVENT_QUEUE);
        let (sends, sends_rx) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared::default());
        tokio::spawn(watch_close(conn.clone(), shared.clone(), events_tx.clone()));
        tokio::spawn(read_datagrams(
            conn.clone(),
            side,
            shared.clone(),
            events_tx.clone(),
        ));
        tokio::spawn(accept_uni(
            conn.clone(),
            side,
            shared.clone(),
            events_tx.clone(),
        ));
        tokio::spawn(write_frames(
            conn.clone(),
            sends_rx,
            streams.control,
            streams.long,
        ));
        if let Some(recv) = streams.control_recv {
            let reader = FrameReader {
                conn: conn.clone(),
                side,
                shared: shared.clone(),
                events: events_tx.clone(),
            };
            tokio::spawn(reader.run(recv, Channel::Control));
        }
        if side == Side::Server {
            let (conn, shared) = (conn.clone(), shared.clone());
            tokio::spawn(async move {
                tokio::time::sleep(HELLO_TIMEOUT).await;
                if !shared.hello_seen.load(Ordering::Acquire) {
                    close_with(&conn, &shared, CloseReason::Timeout);
                }
            });
        }
        QuicConnection {
            conn,
            side,
            events,
            events_tx,
            sends,
            shared,
            closed: false,
            _endpoint: endpoint,
            runtime: None,
        }
    }
}

impl Connection for QuicConnection {
    fn send_reliable(&mut self, channel: Channel, frame: Vec<u8>) -> Result<(), NetError> {
        if self.closed || self.conn.close_reason().is_some() {
            return Err(NetError::Closed);
        }
        self.side.check_send(channel, frame.len())?;
        self.sends
            .send((channel, frame))
            .map_err(|_| NetError::Closed)
    }

    fn send_datagram(&mut self, bytes: Vec<u8>) -> Result<(), NetError> {
        if self.closed {
            return Err(NetError::Closed);
        }
        let size = bytes.len();
        if size > MAX_DATAGRAM {
            return Err(NetError::DatagramTooLarge {
                size,
                max: MAX_DATAGRAM,
            });
        }
        self.conn.send_datagram(bytes.into()).map_err(|e| match e {
            SendDatagramError::TooLarge => NetError::DatagramTooLarge {
                size,
                max: self.conn.max_datagram_size().unwrap_or(0),
            },
            SendDatagramError::ConnectionLost(_) => NetError::Closed,
            other => NetError::Io(other.to_string()),
        })
    }

    fn poll(&mut self, out: &mut Vec<Incoming>) {
        while !self.closed {
            let Ok(event) = self.events.try_recv() else {
                break;
            };
            self.closed = matches!(event, Incoming::Closed(_));
            out.push(event);
        }
    }

    fn rtt(&self) -> Duration {
        self.conn.rtt()
    }

    fn stats(&self) -> LinkStats {
        let s = self.conn.stats();
        LinkStats {
            bytes_sent: s.udp_tx.bytes,
            bytes_received: s.udp_rx.bytes,
            datagrams_dropped: self.shared.datagrams_dropped.load(Ordering::Relaxed),
            datagrams_received: self.shared.datagrams_received.load(Ordering::Relaxed),
        }
    }

    fn close(&mut self, reason: CloseReason) {
        if !self.closed {
            self.closed = true;
            close_with(&self.conn, &self.shared, reason);
        }
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        Some(self.conn.remote_address())
    }
}

impl Drop for QuicConnection {
    fn drop(&mut self) {
        self.close(CloseReason::Normal);
    }
}

/// Reports the close once, with the reason this side gave or the peer's.
async fn watch_close(conn: quinn::Connection, shared: Arc<Shared>, events: mpsc::Sender<Incoming>) {
    let reason = match conn.closed().await {
        ConnectionError::LocallyClosed => shared
            .local_close
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or(CloseReason::Normal),
        ConnectionError::ApplicationClosed(close) => {
            CloseReason::from_wire(close.error_code.into_inner(), &close.reason)
        }
        ConnectionError::TimedOut => CloseReason::Timeout,
        other => CloseReason::TransportError(other.to_string()),
    };
    let _ = events.send(Incoming::Closed(reason)).await;
}

async fn read_datagrams(
    conn: quinn::Connection,
    side: Side,
    shared: Arc<Shared>,
    events: mpsc::Sender<Incoming>,
) {
    let mut limiter = (side == Side::Server).then(DatagramLimiter::default);
    let start = Instant::now();
    while let Ok(bytes) = conn.read_datagram().await {
        let within_rate = limiter.as_mut().is_none_or(|l| l.allow(start.elapsed()));
        if bytes.len() <= MAX_DATAGRAM
            && within_rate
            && events.try_send(Incoming::Datagram(bytes.to_vec())).is_ok()
        {
            shared.datagrams_received.fetch_add(1, Ordering::Relaxed);
        } else {
            shared.datagrams_dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Server: takes the client's one Control stream and hands its send half to the writer.
async fn accept_control(
    conn: quinn::Connection,
    shared: Arc<Shared>,
    events: mpsc::Sender<Incoming>,
    control_tx: oneshot::Sender<SendStream>,
) {
    let mut control_tx = Some(control_tx);
    while let Ok((send, mut recv)) = conn.accept_bi().await {
        let Some(tx) = control_tx.take() else {
            protocol_error(&conn, &shared, "second control stream");
            return;
        };
        match read_tag(&mut recv).await {
            Ok(Some(Channel::Control)) => {}
            Ok(Some(_)) | Err(BadStream) => {
                protocol_error(&conn, &shared, "bad stream tag");
                return;
            }
            Ok(None) => return,
        }
        let _ = tx.send(send);
        let reader = FrameReader {
            conn: conn.clone(),
            side: Side::Server,
            shared: shared.clone(),
            events: events.clone(),
        };
        tokio::spawn(reader.run(recv, Channel::Control));
    }
}

/// Reads the peer's unidirectional streams: Remote on a server; World and Bulk on a client.
async fn accept_uni(
    conn: quinn::Connection,
    side: Side,
    shared: Arc<Shared>,
    events: mpsc::Sender<Incoming>,
) {
    while let Ok(mut recv) = conn.accept_uni().await {
        let reader = FrameReader {
            conn: conn.clone(),
            side,
            shared: shared.clone(),
            events: events.clone(),
        };
        tokio::spawn(async move {
            let allowed = |c: Channel| match side {
                Side::Server => c == Channel::Remote,
                Side::Client => matches!(c, Channel::World | Channel::Bulk),
            };
            match read_tag(&mut recv).await {
                Ok(Some(channel)) if allowed(channel) => reader.run(recv, channel).await,
                Ok(None) => {}
                Ok(Some(_)) | Err(BadStream) => {
                    protocol_error(&reader.conn, &reader.shared, "bad stream tag")
                }
            }
        });
    }
}

/// A stream that broke the framing rules.
struct BadStream;

/// The tag byte; `None` when the stream ended or was reset first.
async fn read_tag(recv: &mut RecvStream) -> Result<Option<Channel>, BadStream> {
    let mut tag = [0u8];
    match recv.read_exact(&mut tag).await {
        Ok(()) => Channel::from_tag(tag[0]).map(Some).ok_or(BadStream),
        Err(_) => Ok(None),
    }
}

struct FrameReader {
    conn: quinn::Connection,
    side: Side,
    shared: Arc<Shared>,
    events: mpsc::Sender<Incoming>,
}

impl FrameReader {
    async fn run(self, mut recv: RecvStream, channel: Channel) {
        let max = self.side.max_incoming_frame();
        loop {
            let len = match read_length(&mut recv).await {
                Ok(Some(len)) => len,
                Ok(None) => return,
                Err(Some(message)) => return protocol_error(&self.conn, &self.shared, message),
                Err(None) => return,
            };
            if len > max {
                let message = format!("frame of {len} bytes is over the {max}-byte limit");
                return protocol_error(&self.conn, &self.shared, message);
            }
            let mut frame = vec![0; len];
            match recv.read_exact(&mut frame).await {
                Ok(()) => {}
                Err(ReadExactError::FinishedEarly(_)) => {
                    return protocol_error(&self.conn, &self.shared, "stream ended inside a frame");
                }
                Err(ReadExactError::ReadError(_)) => return,
            }
            if channel == Channel::Control {
                self.shared.hello_seen.store(true, Ordering::Release);
            }
            if self
                .events
                .send(Incoming::Reliable { channel, frame })
                .await
                .is_err()
            {
                return;
            }
        }
    }
}

/// A LEB128 frame length. `Ok(None)`: the stream ended cleanly between frames. `Err(Some)`: bad framing.
/// `Err(None)`: the stream or connection went away.
async fn read_length(recv: &mut RecvStream) -> Result<Option<usize>, Option<&'static str>> {
    let mut value: u64 = 0;
    for i in 0..5 {
        let mut byte = [0u8];
        match recv.read_exact(&mut byte).await {
            Ok(()) => {}
            Err(ReadExactError::FinishedEarly(_)) if i == 0 => return Ok(None),
            Err(ReadExactError::FinishedEarly(_)) => {
                return Err(Some("stream ended inside a frame length"));
            }
            Err(ReadExactError::ReadError(_)) => return Err(None),
        }
        value |= u64::from(byte[0] & 0x7f) << (7 * i);
        if byte[0] & 0x80 == 0 {
            return usize::try_from(value)
                .map(Some)
                .map_err(|_| Some("frame length too large"));
        }
    }
    Err(Some("frame length too long"))
}

fn encode_frame(frame: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(frame.len() + 5);
    let mut len = frame.len();
    while len >= 0x80 {
        out.push((len as u8 & 0x7f) | 0x80);
        len >>= 7;
    }
    out.push(len as u8);
    out.extend_from_slice(frame);
    out
}

/// Writes queued frames in order per stream. Ends when the connection does.
async fn write_frames(
    conn: quinn::Connection,
    mut sends: mpsc::UnboundedReceiver<(Channel, Vec<u8>)>,
    mut control: ControlSlot,
    mut long: Option<SendStream>,
) {
    while let Some((channel, frame)) = sends.recv().await {
        let bytes = encode_frame(&frame);
        let written = match channel {
            Channel::Bulk => {
                let conn = conn.clone();
                tokio::spawn(async move {
                    let Ok(mut stream) = conn.open_uni().await else {
                        return;
                    };
                    if stream.write_all(&[Channel::Bulk.tag()]).await.is_ok()
                        && stream.write_all(&bytes).await.is_ok()
                    {
                        let _ = stream.finish();
                    }
                });
                Ok(())
            }
            Channel::Control => {
                if let ControlSlot::Waiting(rx) = &mut control {
                    control = match rx.await {
                        Ok(stream) => ControlSlot::Ready(stream),
                        Err(_) => ControlSlot::Gone,
                    };
                }
                match &mut control {
                    ControlSlot::Ready(stream) => stream.write_all(&bytes).await.map_err(|_| ()),
                    _ => Err(()),
                }
            }
            Channel::World | Channel::Remote => match long.as_mut() {
                Some(stream) => stream.write_all(&bytes).await.map_err(|_| ()),
                None => Err(()),
            },
        };
        if written.is_err() {
            return; // The connection is going away; `watch_close` reports why.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::limits::MAX_SERVER_FRAME;

    #[test]
    fn frame_lengths_are_leb128() {
        assert_eq!(encode_frame(&[]), [0]);
        assert_eq!(&encode_frame(&[7; 127])[..1], [127]);
        assert_eq!(&encode_frame(&[7; 128])[..2], [0x80, 1]);
        assert_eq!(
            &encode_frame(&vec![0; MAX_SERVER_FRAME])[..4],
            [0x80, 0x80, 0x80, 2]
        );
    }
}
