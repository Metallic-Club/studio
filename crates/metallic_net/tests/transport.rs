//! Transport tests: in-memory network (manual clock) and QUIC on 127.0.0.1.
//!
//! Every wait has a deadline, so a broken transport fails a test instead of stalling the build.

use std::any::Any;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::mpsc;
use std::thread::sleep;
use std::time::{Duration, Instant};

use metallic_net::NetError;
use metallic_net::transport::limits::{
    IDLE_TIMEOUT, MAX_CLIENT_FRAME, MAX_DATAGRAM, MAX_SERVER_FRAME,
};
use metallic_net::transport::{
    Channel, CloseReason, Connection, Incoming, LinkConditioner, Listener, MemoryNet, QuicClient,
    QuicServer, Sequencer,
};

const TICK: Duration = Duration::from_millis(5);

fn lossy(latency_ms: u64, jitter_ms: u64, loss: f32, seed: u64) -> LinkConditioner {
    LinkConditioner {
        latency: Duration::from_millis(latency_ms),
        jitter: Duration::from_millis(jitter_ms),
        loss,
        seed,
    }
}

/// Connects a client and accepts it on the server side; the client sends its first Control frame (the Hello).
fn memory_pair(
    net: &MemoryNet,
    cond: LinkConditioner,
) -> (Box<dyn Connection>, Box<dyn Connection>) {
    let mut listener = net.listener();
    let mut client = net.connect(cond);
    let server = listener.accept().expect("server side of the connection");
    client
        .send_reliable(Channel::Control, b"hello".to_vec())
        .expect("hello");
    (client, server)
}

fn drain(conn: &mut Box<dyn Connection>) -> Vec<Incoming> {
    let mut out = Vec::new();
    conn.poll(&mut out);
    out
}

fn closed_reason(events: &[Incoming]) -> Option<CloseReason> {
    events.iter().find_map(|e| match e {
        Incoming::Closed(r) => Some(r.clone()),
        _ => None,
    })
}

// ---------------------------------------------------------------- memory

#[test]
fn reliable_stream_in_order_at_30_percent_loss() {
    let net = MemoryNet::new(7);
    let (mut client, mut server) = memory_pair(&net, lossy(40, 10, 0.3, 11));
    let mut up = Vec::new();
    let mut down = Vec::new();
    for i in 0..1000u32 {
        client
            .send_reliable(Channel::Control, i.to_le_bytes().to_vec())
            .unwrap();
        server
            .send_reliable(Channel::World, i.to_le_bytes().to_vec())
            .unwrap();
        net.advance(Duration::from_millis(1));
        collect_frames(&mut server, Channel::Control, &mut up);
        collect_frames(&mut client, Channel::World, &mut down);
    }
    for _ in 0..30_000 {
        if up.len() == 1001 && down.len() == 1000 {
            break;
        }
        net.advance(Duration::from_millis(1));
        collect_frames(&mut server, Channel::Control, &mut up);
        collect_frames(&mut client, Channel::World, &mut down);
    }
    assert_eq!(up.first().map(Vec::as_slice), Some(&b"hello"[..]));
    let up: Vec<u32> = up[1..]
        .iter()
        .map(|f| u32::from_le_bytes(f[..4].try_into().unwrap()))
        .collect();
    let down: Vec<u32> = down
        .iter()
        .map(|f| u32::from_le_bytes(f[..4].try_into().unwrap()))
        .collect();
    assert_eq!(up, (0..1000).collect::<Vec<_>>());
    assert_eq!(down, (0..1000).collect::<Vec<_>>());
}

fn collect_frames(conn: &mut Box<dyn Connection>, want: Channel, into: &mut Vec<Vec<u8>>) {
    for event in drain(conn) {
        match event {
            Incoming::Reliable { channel, frame } => {
                assert_eq!(channel, want);
                into.push(frame);
            }
            Incoming::Datagram(_) => {}
            Incoming::Closed(reason) => panic!("unexpected close: {reason:?}"),
        }
    }
}

#[test]
fn sequencer_is_wrap_aware() {
    let mut s = Sequencer::default();
    assert!(s.accept(65_534));
    assert!(s.accept(65_535));
    assert!(s.accept(0), "0 follows 65,535");
    assert!(!s.accept(65_535), "older after the wrap");
    assert!(!s.accept(0), "duplicate");
    assert!(s.accept(5));
    assert!(!s.accept(3));
}

#[test]
fn stale_datagrams_dropped() {
    let net = MemoryNet::new(3);
    let (mut client, mut server) = memory_pair(&net, lossy(30, 20, 0.05, 5));
    let mut seq: u16 = 65_000;
    let mut sequencer = Sequencer::default();
    let mut accepted = Vec::new();
    let mut rejected = 0;
    let mut handle = |events: Vec<Incoming>, accepted: &mut Vec<u16>, rejected: &mut usize| {
        for event in events {
            if let Incoming::Datagram(bytes) = event {
                let s = u16::from_le_bytes([bytes[0], bytes[1]]);
                if sequencer.accept(s) {
                    accepted.push(s);
                } else {
                    *rejected += 1;
                }
            }
        }
    };
    for _ in 0..1000 {
        server.send_datagram(seq.to_le_bytes().to_vec()).unwrap();
        seq = seq.wrapping_add(1);
        net.advance(Duration::from_millis(2));
        handle(drain(&mut client), &mut accepted, &mut rejected);
    }
    net.advance(Duration::from_millis(500));
    handle(drain(&mut client), &mut accepted, &mut rejected);
    assert!(
        rejected > 0,
        "jitter of 20 ms over 2 ms spacing must reorder some datagrams"
    );
    for pair in accepted.windows(2) {
        assert!(
            (pair[1].wrapping_sub(pair[0]) as i16) > 0,
            "accepted {} after {}",
            pair[1],
            pair[0]
        );
    }
    assert!(
        accepted.iter().any(|&s| s > 65_000),
        "saw seqs before the wrap"
    );
    assert!(accepted.iter().any(|&s| s < 400), "saw seqs after the wrap");
}

#[test]
fn datagram_rate_limited_to_120_per_second() {
    let net = MemoryNet::new(1);
    let (mut client, mut server) = memory_pair(&net, lossy(10, 0, 0.0, 1));
    let mut received = 0;
    for i in 0..200u32 {
        client.send_datagram(i.to_le_bytes().to_vec()).unwrap();
        net.advance(TICK);
        received += drain(&mut server)
            .iter()
            .filter(|e| matches!(e, Incoming::Datagram(_)))
            .count();
    }
    net.advance(Duration::from_millis(20));
    received += drain(&mut server)
        .iter()
        .filter(|e| matches!(e, Incoming::Datagram(_)))
        .count();
    let stats = server.stats();
    assert!(received <= 120, "received {received}");
    assert_eq!(received as u64, stats.datagrams_received);
    assert!(
        stats.datagrams_dropped >= 80,
        "dropped {}",
        stats.datagrams_dropped
    );
    // A second later the window has moved on.
    net.advance(Duration::from_secs(1));
    client.send_datagram(vec![1]).unwrap();
    net.advance(Duration::from_millis(20));
    assert_eq!(drain(&mut server).len(), 1);
}

#[test]
fn oversize_reliable_frame_closes() {
    let net = MemoryNet::new(2);
    let (mut client, mut server) = memory_pair(&net, lossy(20, 0, 0.0, 2));
    client
        .send_reliable(Channel::Control, vec![0; MAX_CLIENT_FRAME])
        .unwrap();
    net.advance(Duration::from_millis(50));
    let events = drain(&mut server);
    assert_eq!(
        events.len(),
        2,
        "hello and the frame at exactly the limit: {events:?}"
    );
    client
        .send_reliable(Channel::Control, vec![0; 70 * 1024])
        .unwrap();
    net.advance(Duration::from_millis(100));
    assert!(matches!(
        closed_reason(&drain(&mut server)),
        Some(CloseReason::ProtocolError(_))
    ));
    assert!(matches!(
        closed_reason(&drain(&mut client)),
        Some(CloseReason::ProtocolError(_))
    ));
    assert_eq!(
        client.send_reliable(Channel::Control, vec![1]),
        Err(NetError::Closed)
    );
    assert_eq!(
        server.send_reliable(Channel::Control, vec![1]),
        Err(NetError::Closed)
    );
}

#[test]
fn datagram_over_1200_refused() {
    let net = MemoryNet::new(4);
    let (mut client, mut server) = memory_pair(&net, LinkConditioner::default());
    assert_eq!(
        client.send_datagram(vec![0; MAX_DATAGRAM + 1]),
        Err(NetError::DatagramTooLarge {
            size: MAX_DATAGRAM + 1,
            max: MAX_DATAGRAM
        })
    );
    assert!(server.send_datagram(vec![0; MAX_DATAGRAM + 1]).is_err());
    client.send_datagram(vec![0; MAX_DATAGRAM]).unwrap();
    net.advance(TICK);
    let events = drain(&mut server);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Incoming::Datagram(d) if d.len() == MAX_DATAGRAM))
    );
}

#[test]
fn channels_only_go_their_way() {
    let net = MemoryNet::new(5);
    let (mut client, mut server) = memory_pair(&net, LinkConditioner::default());
    assert_eq!(
        client.send_reliable(Channel::World, vec![1]),
        Err(NetError::WrongDirection(Channel::World))
    );
    assert_eq!(
        client.send_reliable(Channel::Bulk, vec![1]),
        Err(NetError::WrongDirection(Channel::Bulk))
    );
    assert_eq!(
        server.send_reliable(Channel::Remote, vec![1]),
        Err(NetError::WrongDirection(Channel::Remote))
    );
    client.send_reliable(Channel::Remote, vec![2]).unwrap();
    server.send_reliable(Channel::Bulk, vec![3]).unwrap();
    server.send_reliable(Channel::Control, vec![4]).unwrap();
    net.advance(TICK);
    let up = drain(&mut server);
    assert!(up.contains(&Incoming::Reliable {
        channel: Channel::Remote,
        frame: vec![2]
    }));
    let down = drain(&mut client);
    assert!(down.contains(&Incoming::Reliable {
        channel: Channel::Bulk,
        frame: vec![3]
    }));
    assert!(down.contains(&Incoming::Reliable {
        channel: Channel::Control,
        frame: vec![4]
    }));
}

#[test]
fn hello_must_arrive_within_2_s() {
    let net = MemoryNet::new(6);
    let mut listener = net.listener();
    let mut client = net.connect(lossy(10, 0, 0.0, 6));
    let mut server = listener.accept().unwrap();
    net.advance(Duration::from_millis(1990));
    assert!(drain(&mut server).is_empty());
    net.advance(Duration::from_millis(20));
    assert_eq!(
        closed_reason(&drain(&mut server)),
        Some(CloseReason::Timeout)
    );
    net.advance(Duration::from_millis(20));
    assert_eq!(
        closed_reason(&drain(&mut client)),
        Some(CloseReason::Timeout)
    );
}

#[test]
fn quiet_connection_stays_open_under_loss() {
    let net = MemoryNet::new(8);
    let (mut client, mut server) = memory_pair(&net, lossy(20, 5, 0.1, 8));
    // No application traffic for a minute: keep-alives hold the connection open.
    net.advance(Duration::from_secs(60));
    assert!(closed_reason(&drain(&mut server)).is_none());
    assert!(closed_reason(&drain(&mut client)).is_none());
}

#[test]
fn dead_peer_times_out() {
    let net = MemoryNet::new(10);
    let (client, mut server) = memory_pair(&net, lossy(20, 0, 0.0, 10));
    net.advance(Duration::from_secs(1));
    // The client vanishes without a close (a crashed process). Its last packet, the hello, arrived at 20 ms.
    drop(client);
    net.advance(Duration::from_millis(8_500));
    assert!(closed_reason(&drain(&mut server)).is_none());
    net.advance(Duration::from_secs(1));
    assert_eq!(
        closed_reason(&drain(&mut server)),
        Some(CloseReason::Timeout)
    );
}

#[test]
fn close_reaches_the_peer() {
    let net = MemoryNet::new(9);
    let (mut client, mut server) = memory_pair(&net, lossy(15, 0, 0.0, 9));
    server.close(CloseReason::Kicked("bye".into()));
    assert!(
        drain(&mut server).is_empty(),
        "no events after a local close"
    );
    net.advance(Duration::from_millis(20));
    assert_eq!(
        closed_reason(&drain(&mut client)),
        Some(CloseReason::Kicked("bye".into()))
    );
}

#[test]
fn same_seed_same_arrivals() {
    fn run() -> Vec<u16> {
        let net = MemoryNet::new(42);
        let (mut client, mut server) = memory_pair(&net, lossy(30, 25, 0.2, 42));
        let mut got = Vec::new();
        for s in 0..300u16 {
            server.send_datagram(s.to_le_bytes().to_vec()).unwrap();
            net.advance(Duration::from_millis(3));
            for e in drain(&mut client) {
                if let Incoming::Datagram(d) = e {
                    got.push(u16::from_le_bytes([d[0], d[1]]));
                }
            }
        }
        got
    }
    assert_eq!(run(), run());
}

// ---------------------------------------------------------------- QUIC

const WAIT: Duration = Duration::from_secs(5);

fn localhost() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

fn accept_within(server: &mut QuicServer, limit: Duration) -> Box<dyn Connection> {
    let start = Instant::now();
    loop {
        if let Some(conn) = server.accept() {
            return conn;
        }
        assert!(
            start.elapsed() < limit,
            "no connection accepted within {limit:?}"
        );
        sleep(Duration::from_millis(2));
    }
}

/// Polls until `done` says the collected events are enough, or panics after `limit`.
fn poll_until(
    conn: &mut Box<dyn Connection>,
    limit: Duration,
    mut done: impl FnMut(&[Incoming]) -> bool,
) -> Vec<Incoming> {
    let start = Instant::now();
    let mut all = Vec::new();
    loop {
        conn.poll(&mut all);
        if done(&all) {
            return all;
        }
        assert!(
            start.elapsed() < limit,
            "timed out after {limit:?}; got {all:?}"
        );
        sleep(Duration::from_millis(2));
    }
}

fn quic_pair(
    cond: Option<LinkConditioner>,
) -> (QuicServer, Box<dyn Connection>, Box<dyn Connection>) {
    let mut server = QuicServer::bind(localhost(), cond).expect("bind");
    let mut client = QuicClient::connect(
        server.local_addr(),
        server.cert_sha256(),
        Duration::from_secs(10),
    )
    .expect("connect");
    client
        .send_reliable(Channel::Control, b"hello".to_vec())
        .unwrap();
    let conn = accept_within(&mut server, WAIT);
    (server, client, conn)
}

fn reliable(channel: Channel, frame: &[u8]) -> Incoming {
    Incoming::Reliable {
        channel,
        frame: frame.to_vec(),
    }
}

#[test]
fn quic_round_trip_on_localhost() {
    let (_server, mut client, mut conn) = quic_pair(None);
    assert!(conn.remote_addr().is_some_and(|a| a.ip().is_loopback()));
    client
        .send_reliable(Channel::Remote, b"fire".to_vec())
        .unwrap();
    client.send_datagram(vec![7; MAX_DATAGRAM]).unwrap();
    let up = poll_until(&mut conn, WAIT, |e| e.len() >= 3);
    assert!(up.contains(&reliable(Channel::Control, b"hello")));
    assert!(up.contains(&reliable(Channel::Remote, b"fire")));
    assert!(up.contains(&Incoming::Datagram(vec![7; MAX_DATAGRAM])));

    conn.send_reliable(Channel::Control, b"welcome".to_vec())
        .unwrap();
    conn.send_reliable(Channel::World, b"w1".to_vec()).unwrap();
    conn.send_reliable(Channel::World, b"w2".to_vec()).unwrap();
    conn.send_reliable(Channel::Bulk, vec![9; 300_000]).unwrap();
    conn.send_datagram(vec![1, 2, 3]).unwrap();
    let down = poll_until(&mut client, WAIT, |e| e.len() >= 5);
    assert!(down.contains(&reliable(Channel::Control, b"welcome")));
    let world: Vec<&Incoming> = down
        .iter()
        .filter(|e| {
            matches!(
                e,
                Incoming::Reliable {
                    channel: Channel::World,
                    ..
                }
            )
        })
        .collect();
    assert_eq!(
        world,
        [
            &reliable(Channel::World, b"w1"),
            &reliable(Channel::World, b"w2")
        ]
    );
    assert!(down.contains(&Incoming::Reliable {
        channel: Channel::Bulk,
        frame: vec![9; 300_000]
    }));
    assert!(down.contains(&Incoming::Datagram(vec![1, 2, 3])));

    assert!(matches!(
        client.send_datagram(vec![0; MAX_DATAGRAM + 1]),
        Err(NetError::DatagramTooLarge { .. })
    ));
    assert_eq!(
        client.send_reliable(Channel::World, vec![1]),
        Err(NetError::WrongDirection(Channel::World))
    );
    let stats = client.stats();
    assert!(
        stats.bytes_sent > 0 && stats.bytes_received > 300_000,
        "{stats:?}"
    );

    client.close(CloseReason::Normal);
    let end = poll_until(&mut conn, WAIT, |e| closed_reason(e).is_some());
    assert_eq!(closed_reason(&end), Some(CloseReason::Normal));
}

#[test]
fn quic_wrong_pin_is_refused() {
    let server = QuicServer::bind(localhost(), None).unwrap();
    let mut pin = server.cert_sha256();
    pin[0] ^= 1;
    let result = QuicClient::connect(server.local_addr(), pin, WAIT);
    assert!(
        matches!(result, Err(NetError::Connect(_))),
        "{:?}",
        result.err()
    );
}

#[test]
fn quic_fifth_client_stream_refused() {
    let mut server = QuicServer::bind(localhost(), None).unwrap();
    let config = QuicClient::quinn_config(server.cert_sha256()).unwrap();
    let addr = server.local_addr();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let streams = rt.block_on(async {
        let endpoint = quinn::Endpoint::client(localhost()).unwrap();
        let conn = endpoint
            .connect_with(config, addr, "metallic")
            .unwrap()
            .await
            .unwrap();
        // Control: tag 0, then one frame (length 5, "hello").
        let (mut control, _recv) = conn.open_bi().await.unwrap();
        control.write_all(&[0, 5]).await.unwrap();
        control.write_all(b"hello").await.unwrap();
        let mut open = vec![control];
        // One Remote stream (tag 2, one frame) and two streams opened but never written: four open in total.
        let mut remote = conn.open_uni().await.unwrap();
        remote.write_all(&[2, 1, 42]).await.unwrap();
        open.push(remote);
        for _ in 0..2 {
            open.push(conn.open_uni().await.unwrap());
        }
        let fifth_uni = tokio::time::timeout(Duration::from_millis(500), conn.open_uni()).await;
        let second_bi = tokio::time::timeout(Duration::from_millis(200), conn.open_bi()).await;
        (fifth_uni.is_err(), second_bi.is_err(), open, conn, endpoint)
    });
    assert!(streams.0, "a fifth stream must wait for QUIC stream credit");
    assert!(streams.1, "a second bidirectional stream must wait too");
    let mut conn = accept_within(&mut server, WAIT);
    let events = poll_until(&mut conn, WAIT, |e| e.len() >= 2);
    assert_eq!(
        events,
        [
            reliable(Channel::Control, b"hello"),
            reliable(Channel::Remote, &[42])
        ]
    );
    drop(streams);
    drop(rt);
}

#[test]
fn quic_oversize_reliable_frame_closes() {
    let (_server, mut client, mut conn) = quic_pair(None);
    client
        .send_reliable(Channel::Control, vec![0; 70 * 1024])
        .unwrap();
    let down = poll_until(&mut client, WAIT, |e| closed_reason(e).is_some());
    assert!(
        matches!(closed_reason(&down), Some(CloseReason::ProtocolError(_))),
        "{down:?}"
    );
    let up = poll_until(&mut conn, WAIT, |e| closed_reason(e).is_some());
    assert!(matches!(
        closed_reason(&up),
        Some(CloseReason::ProtocolError(_))
    ));
    assert!(
        !up.iter().any(
            |e| matches!(e, Incoming::Reliable { frame, .. } if frame.len() > MAX_CLIENT_FRAME)
        )
    );
}

#[test]
fn quic_hello_must_arrive_within_2_s() {
    let mut server = QuicServer::bind(localhost(), None).unwrap();
    let start = Instant::now();
    let mut client = QuicClient::connect(server.local_addr(), server.cert_sha256(), WAIT).unwrap();
    let mut conn = accept_within(&mut server, WAIT);
    let down = poll_until(&mut client, WAIT, |e| closed_reason(e).is_some());
    assert_eq!(closed_reason(&down), Some(CloseReason::Timeout));
    let elapsed = start.elapsed();
    assert!(
        elapsed >= Duration::from_millis(1900) && elapsed < Duration::from_secs(4),
        "{elapsed:?}"
    );
    let up = poll_until(&mut conn, WAIT, |e| closed_reason(e).is_some());
    assert_eq!(closed_reason(&up), Some(CloseReason::Timeout));
}

#[test]
fn quic_datagram_rate_limited() {
    let (_server, mut client, mut conn) = quic_pair(None);
    for i in 0..200u32 {
        client.send_datagram(i.to_le_bytes().to_vec()).unwrap();
    }
    let start = Instant::now();
    let mut received = 0;
    while start.elapsed() < Duration::from_millis(800) {
        let mut events = Vec::new();
        conn.poll(&mut events);
        received += events
            .iter()
            .filter(|e| matches!(e, Incoming::Datagram(_)))
            .count();
        if conn.stats().datagrams_received + conn.stats().datagrams_dropped >= 200 {
            break;
        }
        sleep(Duration::from_millis(2));
    }
    let mut events = Vec::new();
    conn.poll(&mut events);
    received += events
        .iter()
        .filter(|e| matches!(e, Incoming::Datagram(_)))
        .count();
    let stats = conn.stats();
    assert!(received <= 120, "received {received}");
    assert_eq!(received as u64, stats.datagrams_received);
    assert!(stats.datagrams_dropped >= 60, "{stats:?}");
}

#[test]
fn quic_conditioner_adds_latency_and_loss() {
    let cond = lossy(50, 5, 0.1, 99);
    let (_server, mut client, mut conn) = quic_pair(Some(cond));
    for i in 0..50u8 {
        client.send_reliable(Channel::Control, vec![i]).unwrap();
    }
    let up = poll_until(&mut conn, Duration::from_secs(10), |e| e.len() >= 51);
    let frames: Vec<u8> = up[1..]
        .iter()
        .map(|e| match e {
            Incoming::Reliable { frame, .. } => frame[0],
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(frames, (0..50).collect::<Vec<_>>());
    assert!(
        client.rtt() >= Duration::from_millis(80),
        "rtt {:?}",
        client.rtt()
    );
}

#[test]
fn quic_idle_timeout_closes_a_vanished_client() {
    let mut server = QuicServer::bind(localhost(), None).unwrap();
    let config = QuicClient::quinn_config(server.cert_sha256()).unwrap();
    let addr = server.local_addr();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let held = rt.block_on(async {
        let endpoint = quinn::Endpoint::client(localhost()).unwrap();
        let conn = endpoint
            .connect_with(config, addr, "metallic")
            .unwrap()
            .await
            .unwrap();
        let (mut control, _recv) = conn.open_bi().await.unwrap();
        control.write_all(&[0, 5]).await.unwrap();
        control.write_all(b"hello").await.unwrap();
        // Drive the client long enough for the hello to go out.
        tokio::time::sleep(Duration::from_millis(300)).await;
        (endpoint, conn, control)
    });
    let mut conn = accept_within(&mut server, WAIT);
    let first = poll_until(&mut conn, WAIT, |e| !e.is_empty());
    assert_eq!(first, [reliable(Channel::Control, b"hello")]);
    // The client process "dies": its runtime stops without sending a close.
    let start = Instant::now();
    rt.shutdown_background();
    std::mem::forget(held);
    let end = poll_until(&mut conn, IDLE_TIMEOUT + Duration::from_secs(5), |e| {
        closed_reason(e).is_some()
    });
    assert_eq!(closed_reason(&end), Some(CloseReason::Timeout));
    assert!(
        start.elapsed() >= IDLE_TIMEOUT - Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
}

// ---------------------------------------------------------------- raw QUIC clients (hostile or stalled peers)

/// Things a raw-client script keeps open until the connection closes.
type Keep = Vec<Box<dyn Any + Send>>;
type ScriptFuture = Pin<Box<dyn Future<Output = Keep> + Send>>;
type Script =
    Box<dyn FnOnce(quinn::Connection, quinn::SendStream, quinn::RecvStream) -> ScriptFuture + Send>;

/// Runs a raw quinn client on its own thread: connects, sends a Hello on Control, runs `script` with the connection
/// and the Control stream halves, then keeps the connection driven until it closes (at most 20 s). Sends back the
/// application close code the server used, or how else the connection ended.
fn raw_client(server: &QuicServer, script: Script) -> mpsc::Receiver<Result<u64, String>> {
    let config = QuicClient::quinn_config(server.cert_sha256()).unwrap();
    let addr = server.local_addr();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let endpoint = quinn::Endpoint::client(localhost()).unwrap();
            let conn = endpoint
                .connect_with(config, addr, "metallic")
                .unwrap()
                .await
                .unwrap();
            let (mut control, recv) = conn.open_bi().await.unwrap();
            control.write_all(&[0, 5]).await.unwrap();
            control.write_all(b"hello").await.unwrap();
            let _keep = script(conn.clone(), control, recv).await;
            let code = match tokio::time::timeout(Duration::from_secs(20), conn.closed()).await {
                Ok(quinn::ConnectionError::ApplicationClosed(close)) => {
                    Ok(close.error_code.into_inner())
                }
                Ok(other) => Err(format!("{other:?}")),
                Err(_) => Err("still open".into()),
            };
            let _ = tx.send(code);
        });
    });
    rx
}

fn keep_control() -> Script {
    Box::new(|_, control, recv| {
        Box::pin(async move { vec![Box::new(control) as Box<dyn Any + Send>, Box::new(recv)] })
    })
}

/// A script that also opens one unidirectional stream and writes `bytes` to it (finishing it if `finish`).
fn uni_stream(bytes: &'static [u8], finish: bool) -> Script {
    Box::new(move |conn, control, recv| {
        Box::pin(async move {
            let mut s = conn.open_uni().await.unwrap();
            s.write_all(bytes).await.unwrap();
            if finish {
                s.finish().unwrap();
            }
            vec![
                Box::new(control) as Box<dyn Any + Send>,
                Box::new(recv),
                Box::new(s),
            ]
        })
    })
}

const PROTOCOL_ERROR: u64 = 2;
const TRANSPORT_ERROR: u64 = 4;

/// Accepts the raw client and waits for the server side to close; returns the reason.
fn server_close_reason(server: &mut QuicServer) -> CloseReason {
    let mut conn = accept_within(server, WAIT);
    let events = poll_until(&mut conn, WAIT, |e| closed_reason(e).is_some());
    closed_reason(&events).unwrap()
}

#[test]
fn quic_second_remote_stream_closes() {
    let mut server = QuicServer::bind(localhost(), None).unwrap();
    let script: Script = Box::new(|conn, control, recv| {
        Box::pin(async move {
            let mut keep: Keep = vec![Box::new(control), Box::new(recv)];
            for _ in 0..2 {
                let mut s = conn.open_uni().await.unwrap();
                s.write_all(&[2, 1, 42]).await.unwrap();
                keep.push(Box::new(s));
            }
            keep
        })
    });
    let code = raw_client(&server, script);
    assert!(matches!(
        server_close_reason(&mut server),
        CloseReason::ProtocolError(_)
    ));
    assert_eq!(
        code.recv_timeout(Duration::from_secs(25)).unwrap(),
        Ok(PROTOCOL_ERROR)
    );
}

#[test]
fn quic_second_control_stream_closes() {
    let mut server = QuicServer::bind(localhost(), None).unwrap();
    // Finish and stop the first Control stream so QUIC gives the credit back, then open another one.
    let script: Script = Box::new(|conn, mut control, mut recv| {
        Box::pin(async move {
            control.finish().unwrap();
            recv.stop(quinn::VarInt::from_u32(0)).unwrap();
            let second = tokio::time::timeout(Duration::from_secs(5), conn.open_bi()).await;
            let mut keep: Keep = vec![Box::new(control), Box::new(recv)];
            if let Ok(Ok((mut send, recv2))) = second {
                send.write_all(&[0, 1, 7]).await.unwrap();
                keep.push(Box::new(send));
                keep.push(Box::new(recv2));
            }
            keep
        })
    });
    let code = raw_client(&server, script);
    assert!(matches!(
        server_close_reason(&mut server),
        CloseReason::ProtocolError(_)
    ));
    assert_eq!(
        code.recv_timeout(Duration::from_secs(25)).unwrap(),
        Ok(PROTOCOL_ERROR)
    );
}

#[test]
fn quic_bad_stream_tag_closes() {
    // A client may only open Remote (2): World (1), Bulk (3) and unknown tags close the connection.
    for bytes in [&[1u8, 1, 0][..], &[3, 1, 0], &[9, 1, 0]] {
        let mut server = QuicServer::bind(localhost(), None).unwrap();
        let code = raw_client(&server, uni_stream(bytes, false));
        assert!(matches!(
            server_close_reason(&mut server),
            CloseReason::ProtocolError(_)
        ));
        assert_eq!(
            code.recv_timeout(Duration::from_secs(25)).unwrap(),
            Ok(PROTOCOL_ERROR)
        );
    }
}

#[test]
fn quic_stream_ending_mid_frame_closes() {
    // Ends inside a frame body (10 bytes announced, 3 sent) and inside a length (continuation bit set).
    for bytes in [&[2u8, 10, 1, 2, 3][..], &[2, 0x80]] {
        let mut server = QuicServer::bind(localhost(), None).unwrap();
        let code = raw_client(&server, uni_stream(bytes, true));
        assert!(matches!(
            server_close_reason(&mut server),
            CloseReason::ProtocolError(_)
        ));
        assert_eq!(
            code.recv_timeout(Duration::from_secs(25)).unwrap(),
            Ok(PROTOCOL_ERROR)
        );
    }
}

#[test]
fn quic_stalled_reader_is_closed() {
    let mut server = QuicServer::bind(localhost(), None).unwrap();
    // The client acks and keeps the connection alive but never reads World.
    let code = raw_client(&server, keep_control());
    let mut conn = accept_within(&mut server, WAIT);
    poll_until(&mut conn, WAIT, |e| !e.is_empty());
    let frame = vec![5u8; 1 << 20];
    let mut refused_after = None;
    for i in 0..64 {
        if conn.send_reliable(Channel::World, frame.clone()).is_err() {
            refused_after = Some(i);
            break;
        }
        sleep(Duration::from_millis(5));
    }
    let refused_after = refused_after.expect("sends past the budget must fail");
    assert!(
        refused_after * (1 << 20) <= 2 * MAX_SERVER_FRAME,
        "{refused_after}"
    );
    let events = poll_until(&mut conn, WAIT, |e| closed_reason(e).is_some());
    assert!(
        matches!(closed_reason(&events), Some(CloseReason::TransportError(_))),
        "{events:?}"
    );
    // The close usually arrives; if the client's socket dropped it under the flood, the client gets a stateless
    // reset once the server has forgotten the connection.
    let end = code.recv_timeout(Duration::from_secs(25)).unwrap();
    assert!(
        end == Ok(TRANSPORT_ERROR) || end == Err("Reset".into()),
        "{end:?}"
    );
}

#[test]
fn quic_stalled_world_does_not_block_control() {
    let mut server = QuicServer::bind(localhost(), None).unwrap();
    let (got_tx, got_rx) = mpsc::channel();
    // The client reads Control but never World.
    let script: Script = Box::new(move |_, control, mut recv| {
        Box::pin(async move {
            let mut buf = [0u8; 2];
            let read =
                tokio::time::timeout(Duration::from_secs(10), recv.read_exact(&mut buf)).await;
            let _ = got_tx.send(matches!(read, Ok(Ok(()))) && buf == [1, 7]);
            vec![Box::new(control) as Box<dyn Any + Send>, Box::new(recv)]
        })
    });
    let _code = raw_client(&server, script);
    let mut conn = accept_within(&mut server, WAIT);
    poll_until(&mut conn, WAIT, |e| !e.is_empty());
    for _ in 0..3 {
        conn.send_reliable(Channel::World, vec![5u8; 1 << 20])
            .unwrap();
    }
    sleep(Duration::from_millis(200));
    conn.send_reliable(Channel::Control, vec![7]).unwrap();
    assert!(
        got_rx.recv_timeout(Duration::from_secs(15)).unwrap(),
        "Control frame arrived"
    );
}
