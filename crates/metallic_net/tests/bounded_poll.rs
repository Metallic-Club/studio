//! Bounded delivery through the public memory and QUIC Connection APIs.
use metallic_net::transport::{
    Channel, CloseReason, Connection, Incoming, LinkConditioner, Listener, MemoryNet, PollBudget,
    PollReport, PollStop, QuicClient, QuicServer,
};
use std::time::{Duration, Instant};

fn budget(max_events: usize, max_bytes: usize) -> PollBudget {
    PollBudget {
        max_events,
        max_bytes,
    }
}

fn reliable(bytes: &[u8]) -> Incoming {
    Incoming::Reliable {
        channel: Channel::Control,
        frame: bytes.to_vec(),
    }
}

fn memory_pair() -> (MemoryNet, Box<dyn Connection>, Box<dyn Connection>) {
    let net = MemoryNet::new(430);
    let mut listener = net.listener();
    let client = net.connect(LinkConditioner::default());
    let server = listener.accept().unwrap();
    (net, client, server)
}

#[test]
fn memory_partial_drains_preserve_fifo_bytes_empty_events_and_existing_output() {
    let (net, mut client, mut server) = memory_pair();
    client.send_reliable(Channel::Control, vec![1; 3]).unwrap();
    client.send_datagram(Vec::new()).unwrap();
    client.send_reliable(Channel::Control, vec![2; 2]).unwrap();
    client.send_datagram(vec![3; 4]).unwrap();
    net.advance(Duration::ZERO);
    let stats = server.stats();
    let prefix = Incoming::Datagram(vec![99; 100]);
    let mut out = vec![prefix.clone()];
    assert_eq!(
        server.poll_bounded(&mut out, PollBudget::default()).stop,
        PollStop::EventLimit
    );
    assert_eq!(
        server.poll_bounded(&mut out, budget(2, 3)),
        PollReport {
            events: 2,
            bytes: 3,
            stop: PollStop::EventLimit
        }
    );
    assert_eq!(
        server.poll_bounded(&mut out, budget(8, 1)),
        PollReport {
            events: 0,
            bytes: 0,
            stop: PollStop::ByteLimit {
                next_event_bytes: 2
            }
        }
    );
    assert_eq!(
        server.poll_bounded(&mut out, budget(8, 2)),
        PollReport {
            events: 1,
            bytes: 2,
            stop: PollStop::ByteLimit {
                next_event_bytes: 4
            }
        }
    );
    assert_eq!(
        server.poll_bounded(&mut out, budget(8, 4)),
        PollReport {
            events: 1,
            bytes: 4,
            stop: PollStop::Empty
        }
    );
    assert_eq!(
        out,
        [
            prefix,
            reliable(&[1; 3]),
            Incoming::Datagram(Vec::new()),
            reliable(&[2; 2]),
            Incoming::Datagram(vec![3; 4])
        ]
    );
    assert_eq!(
        server.stats(),
        stats,
        "polling must not double-count transport receipt"
    );
}

#[test]
fn memory_deferred_close_counts_utf8_once_and_legacy_poll_keeps_the_head() {
    let (net, mut client, mut server) = memory_pair();
    client.send_reliable(Channel::Control, vec![1; 5]).unwrap();
    client.close(CloseReason::Kicked("é".into()));
    net.advance(Duration::ZERO);
    let mut out = Vec::new();
    for _ in 0..2 {
        assert_eq!(
            server.poll_bounded(&mut out, budget(9, 4)),
            PollReport {
                events: 0,
                bytes: 0,
                stop: PollStop::ByteLimit {
                    next_event_bytes: 5
                }
            }
        );
    }
    assert_eq!(
        server.poll_bounded(&mut out, budget(9, 5)),
        PollReport {
            events: 1,
            bytes: 5,
            stop: PollStop::ByteLimit {
                next_event_bytes: 2
            }
        }
    );
    assert_eq!(
        server.poll_bounded(&mut out, budget(0, usize::MAX)).events,
        0
    );
    assert_eq!(
        server.poll_bounded(&mut out, budget(1, 1)).stop,
        PollStop::ByteLimit {
            next_event_bytes: 2
        }
    );
    server.poll(&mut out);
    assert_eq!(
        out,
        [
            reliable(&[1; 5]),
            Incoming::Closed(CloseReason::Kicked("é".into()))
        ]
    );
    assert_eq!(server.poll_bounded(&mut out, budget(9, 99)).events, 0);
}

#[test]
fn memory_local_close_discards_a_retained_head_and_queued_remote_close() {
    let (net, mut client, mut server) = memory_pair();
    client.send_reliable(Channel::Control, vec![1; 5]).unwrap();
    client.close(CloseReason::Normal);
    net.advance(Duration::ZERO);
    let mut out = Vec::new();
    assert_eq!(
        server.poll_bounded(&mut out, budget(1, 0)).stop,
        PollStop::ByteLimit {
            next_event_bytes: 5
        }
    );
    server.close(CloseReason::Normal);
    server.poll(&mut out);
    assert_eq!(
        server.poll_bounded(&mut out, budget(9, 99)),
        PollReport {
            events: 0,
            bytes: 0,
            stop: PollStop::Closed
        }
    );
    assert!(out.is_empty());
}

fn wait_for(
    connection: &mut dyn Connection,
    out: &mut Vec<Incoming>,
    limits: PollBudget,
    done: impl Fn(PollReport) -> bool,
) -> PollReport {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let before = out.len();
        let report = connection.poll_bounded(out, limits);
        assert_eq!(out.len() - before, report.events);
        assert!(report.events <= limits.max_events && report.bytes <= limits.max_bytes);
        if done(report) {
            return report;
        }
        assert!(
            Instant::now() < deadline,
            "bounded QUIC receive did not reach the fixture state: {report:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn quic_public_api_retains_large_head_and_legacy_poll_preserves_fifo() {
    let mut listener = QuicServer::bind("127.0.0.1:0".parse().unwrap(), None).unwrap();
    let mut client = QuicClient::connect(
        listener.local_addr(),
        listener.cert_sha256(),
        Duration::from_secs(5),
    )
    .unwrap();
    client.send_reliable(Channel::Control, vec![1; 10]).unwrap();
    client.send_reliable(Channel::Control, Vec::new()).unwrap();
    client.send_reliable(Channel::Control, vec![2; 2]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut server = loop {
        if let Some(conn) = listener.accept() {
            break conn;
        }
        assert!(Instant::now() < deadline, "QUIC fixture did not connect");
        std::thread::sleep(Duration::from_millis(2));
    };
    let prefix = Incoming::Datagram(vec![99; 100]);
    let mut out = vec![prefix.clone()];
    assert_eq!(
        server.poll_bounded(&mut out, budget(0, usize::MAX)).events,
        0
    );
    let report = wait_for(&mut *server, &mut out, budget(1, 3), |r| {
        matches!(r.stop, PollStop::ByteLimit { .. })
    });
    assert_eq!(
        report,
        PollReport {
            events: 0,
            bytes: 0,
            stop: PollStop::ByteLimit {
                next_event_bytes: 10
            }
        }
    );
    assert_eq!(out, [prefix.clone()]);
    assert_eq!(
        server.poll_bounded(&mut out, budget(1, 10)),
        PollReport {
            events: 1,
            bytes: 10,
            stop: PollStop::EventLimit
        }
    );
    assert_eq!(
        wait_for(&mut *server, &mut out, budget(1, 0), |r| r.events == 1),
        PollReport {
            events: 1,
            bytes: 0,
            stop: PollStop::EventLimit
        }
    );
    // Retain the final head, then use the old API: it must still flush that head first.
    wait_for(&mut *server, &mut out, budget(1, 1), |r| {
        matches!(r.stop, PollStop::ByteLimit { .. })
    });
    server.poll(&mut out);
    assert_eq!(
        out,
        [prefix, reliable(&[1; 10]), reliable(&[]), reliable(&[2; 2])]
    );
    assert_eq!(
        server.poll_bounded(&mut out, budget(8, 99)).stop,
        PollStop::Empty
    );
}
