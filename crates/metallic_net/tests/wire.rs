//! Wire format: messages, frames, datagrams, bit packing, quantization and value conversion (task A6).

use metallic_core::{CFrame, Color3, UDim2, Value, ValueType, Vec3, World};
use metallic_net::transport::limits::{MAX_CLIENT_FRAME, MAX_DATAGRAM};
use metallic_net::wire::*;
use proptest::prelude::*;

// ---------- strategies ----------

fn finite() -> impl Strategy<Value = f64> {
    -1.0e9f64..1.0e9
}

fn finite32() -> impl Strategy<Value = f32> {
    -1.0e6f32..1.0e6
}

fn net_id() -> impl Strategy<Value = NetId> {
    prop_oneof![1u64..64, 64u64..100_000, any::<u64>()].prop_map(NetId)
}

fn net_value() -> impl Strategy<Value = NetValue> {
    let leaf = prop_oneof![
        Just(NetValue::Nil),
        any::<bool>().prop_map(NetValue::Bool),
        finite().prop_map(NetValue::Number),
        proptest::collection::vec(any::<u8>(), 0..24).prop_map(NetValue::Str),
        (finite(), finite(), finite()).prop_map(|(x, y, z)| NetValue::Vector3(Vec3::new(x, y, z))),
        (finite(), finite()).prop_map(|(x, y)| NetValue::Vector2([x, y])),
        (0.0f64..255.0, 0.0f64..255.0, 0.0f64..255.0)
            .prop_map(|(r, g, b)| NetValue::Color3(Color3::new(r, g, b))),
        (finite(), finite(), finite(), finite())
            .prop_map(|(a, b, c, d)| NetValue::UDim2(UDim2::new(a, b, c, d))),
        proptest::option::of(net_id()).prop_map(NetValue::Ref),
        (
            finite(),
            finite(),
            finite(),
            [finite(), finite(), finite(), finite()]
        )
            .prop_map(|(x, y, z, rotation)| NetValue::CFrame(Box::new(NetCFrame {
                position: Vec3::new(x, y, z),
                rotation,
            }))),
    ];
    leaf.prop_recursive(4, 32, 6, |inner| {
        proptest::collection::vec((inner.clone(), inner), 0..6).prop_map(NetValue::Table)
    })
}

fn args() -> impl Strategy<Value = Vec<NetValue>> {
    proptest::collection::vec(net_value(), 0..5)
}

fn props() -> impl Strategy<Value = Vec<(u16, NetValue)>> {
    proptest::collection::vec((any::<u16>(), net_value()), 0..5)
}

fn create() -> impl Strategy<Value = Create> {
    (
        net_id(),
        any::<u16>(),
        proptest::option::of(net_id()),
        props(),
    )
        .prop_map(|(net, class, parent, props)| Create {
            net,
            class,
            parent,
            props,
        })
}

fn reply() -> impl Strategy<Value = Result<Vec<NetValue>, String>> {
    prop_oneof![args().prop_map(Ok), ".{0,20}".prop_map(Err)]
}

fn client_msg() -> impl Strategy<Value = ClientMsg> {
    prop_oneof![
        (".{0,40}", any::<u16>(), ".{0,12}", any::<[u8; 32]>()).prop_map(
            |(ticket, protocol_minor, engine_version, registry_hash)| ClientMsg::Hello {
                ticket,
                protocol_minor,
                engine_version,
                registry_hash,
            }
        ),
        (net_id(), args()).prop_map(|(remote, args)| ClientMsg::FireServer { remote, args }),
        (any::<u32>(), net_id(), args())
            .prop_map(|(call, remote, args)| { ClientMsg::InvokeServer { call, remote, args } }),
        (any::<u32>(), reply())
            .prop_map(|(call, result)| ClientMsg::InvokeClientReply { call, result }),
        Just(ClientMsg::Bye),
    ]
}

fn server_msg() -> impl Strategy<Value = ServerMsg> {
    prop_oneof![
        (net_id(), any::<u64>(), any::<u16>()).prop_map(|(player, tick, protocol_minor)| {
            ServerMsg::Welcome {
                player,
                tick,
                protocol_minor,
            }
        }),
        any::<[u8; 32]>().prop_map(|registry_hash| ServerMsg::UpdateRequired { registry_hash }),
        ".{0,40}".prop_map(|reason| ServerMsg::Kick { reason }),
        (any::<u64>(), proptest::collection::vec(any::<u8>(), 0..64))
            .prop_map(|(tick, zstd)| ServerMsg::JoinSnapshot { tick, zstd }),
        create().prop_map(ServerMsg::Create),
        net_id().prop_map(|net| ServerMsg::Destroy { net }),
        (net_id(), proptest::option::of(net_id()))
            .prop_map(|(net, parent)| ServerMsg::Reparent { net, parent }),
        (net_id(), props()).prop_map(|(net, props)| ServerMsg::Set { net, props }),
        (net_id(), args()).prop_map(|(remote, args)| ServerMsg::FireClient { remote, args }),
        (any::<u32>(), net_id(), args())
            .prop_map(|(call, remote, args)| { ServerMsg::InvokeClient { call, remote, args } }),
        (any::<u32>(), reply())
            .prop_map(|(call, result)| ServerMsg::InvokeServerReply { call, result }),
    ]
}

fn input_packet() -> impl Strategy<Value = InputPacket> {
    (
        any::<u16>(),
        any::<u16>(),
        any::<u32>(),
        0u32..u32::MAX - 8,
        proptest::collection::vec(
            (
                any::<i8>(),
                any::<i8>(),
                any::<bool>(),
                any::<bool>(),
                any::<u16>(),
                any::<u16>(),
            ),
            1..=8,
        ),
    )
        .prop_map(|(seq, ack, ack_bits, first, raw)| InputPacket {
            seq,
            ack,
            ack_bits,
            cmds: raw
                .into_iter()
                .enumerate()
                .map(|(i, (move_x, move_z, jump, run, yaw, actions))| InputCmd {
                    tick: first + i as u32,
                    move_x,
                    move_z,
                    jump,
                    run,
                    yaw,
                    actions,
                })
                .collect(),
        })
}

fn quant_pos() -> impl Strategy<Value = QuantPos> {
    (
        -100_000.0f64..100_000.0,
        -20_000.0f64..20_000.0,
        -100_000.0f64..100_000.0,
    )
        .prop_map(|(x, y, z)| quantize_position(Vec3::new(x, y, z)))
}

fn own_state() -> impl Strategy<Value = OwnState> {
    (
        [finite32(), finite32(), finite32()],
        finite32(),
        [finite32(), finite32(), finite32()],
        any::<u8>(),
        proptest::option::of(net_id()),
        finite32(),
        finite32(),
        [finite32(), finite32(), finite32()],
        [finite32(), finite32(), finite32()],
    )
        .prop_map(
            |(position, vertical, inherited, state, ground, yaw, dead_time, velocity, relative)| {
                OwnState {
                    position,
                    vertical,
                    inherited,
                    state,
                    ground,
                    yaw,
                    dead_time,
                    velocity,
                    relative,
                }
            },
        )
}

fn entity() -> impl Strategy<Value = EntityState> {
    (
        net_id(),
        quant_pos(),
        prop_oneof![
            any::<u32>().prop_map(EntityRot::Part),
            (any::<u16>(), any::<u8>())
                .prop_map(|(yaw, state)| EntityRot::Character { yaw, state }),
        ],
        proptest::option::of(
            (-2048i16..2048, -2048i16..2048, -2048i16..2048).prop_map(|(a, b, c)| [a, b, c]),
        ),
        any::<bool>(),
    )
        .prop_map(|(net, pos, rot, vel, at_rest)| EntityState {
            net,
            pos,
            rot,
            vel,
            at_rest,
        })
}

fn snapshot_packet() -> impl Strategy<Value = SnapshotPacket> {
    (
        any::<u16>(),
        any::<u8>(),
        any::<u32>(),
        any::<u32>(),
        any::<i8>(),
        proptest::option::of(own_state()),
        proptest::collection::vec(entity(), 0..20),
    )
        .prop_map(
            |(seq, source, server_tick, last_input_tick, buffer_hint, own, entities)| {
                SnapshotPacket {
                    seq,
                    source,
                    server_tick,
                    last_input_tick,
                    buffer_hint,
                    own,
                    entities,
                }
            },
        )
}

fn unreliable() -> impl Strategy<Value = UnreliableRemotePacket> {
    (any::<u16>(), net_id(), args()).prop_map(|(seq, remote, args)| UnreliableRemotePacket {
        seq,
        remote,
        args,
    })
}

/// Every decoder on one input; none may panic. Returns how many accepted it.
fn run_all_decoders(bytes: &[u8]) -> usize {
    [
        decode_client(bytes).is_ok(),
        decode_server(bytes).is_ok(),
        decode_snapshot(bytes).is_ok(),
        decode_input(bytes).is_ok(),
        decode_unreliable_remote(bytes).is_ok(),
        datagram_kind(bytes).is_ok(),
        decompress_join(bytes).is_ok(),
    ]
    .iter()
    .filter(|ok| **ok)
    .count()
}

fn rotation_angle(a: Vec3, b: Vec3) -> f64 {
    let ca = CFrame::from_position_orientation(Vec3::default(), a);
    let cb = CFrame::from_position_orientation(Vec3::default(), b);
    ca.inverse().to_world(cb).to_axis_angle().1.abs()
}

// ---------- tests ----------

proptest! {
    #[test]
    fn server_and_client_messages_round_trip(c in client_msg(), s in server_msg(), source in any::<u8>()) {
        let bytes = encode_client(&c).unwrap();
        prop_assert_eq!(decode_client(&bytes).unwrap(), Decoded::Msg { source: 0, msg: c });
        let bytes = encode_server(&s).unwrap();
        prop_assert_eq!(bytes[1], 0);
        let mut relayed = bytes.clone();
        relayed[1] = source;
        prop_assert_eq!(decode_server(&relayed).unwrap(), Decoded::Msg { source, msg: s });
    }

    #[test]
    fn join_snapshot_round_trips(creates in proptest::collection::vec(create(), 0..20)) {
        let zstd = compress_join(&creates).unwrap();
        prop_assert_eq!(decompress_join(&zstd).unwrap(), creates);
    }

    #[test]
    fn bits_round_trip(fields in proptest::collection::vec((any::<u64>(), 1u8..=64), 0..40), tail in any::<bool>()) {
        let mut w = BitWriter::new();
        for &(v, bits) in &fields {
            w.write(v, bits);
        }
        w.write_bool(tail);
        let bytes = w.finish();
        let total: usize = fields.iter().map(|f| f.1 as usize).sum::<usize>() + 1;
        prop_assert_eq!(bytes.len(), total.div_ceil(8));
        let mut r = BitReader::new(&bytes);
        for &(v, bits) in &fields {
            let mask = if bits == 64 { u64::MAX } else { (1u64 << bits) - 1 };
            prop_assert_eq!(r.read(bits).unwrap(), v & mask);
        }
        prop_assert_eq!(r.read_bool().unwrap(), tail);
        prop_assert!(r.finish().is_ok());
    }

    #[test]
    fn varint_round_trip(u in any::<u64>(), i in any::<i64>()) {
        let mut out = Vec::new();
        write_u64(&mut out, u);
        write_i64(&mut out, i);
        let (got_u, n) = read_u64(&out).unwrap();
        prop_assert_eq!(got_u, u);
        let (got_i, m) = read_i64(&out[n..]).unwrap();
        prop_assert_eq!(got_i, i);
        prop_assert_eq!(n + m, out.len());
        prop_assert!(read_u64(&out[..n - 1]).is_err());
    }

    #[test]
    fn position_error_at_most_1_128(
        x in -100_000.0f64..100_000.0,
        y in -8_000.0f64..8_000.0,
        z in -100_000.0f64..100_000.0,
    ) {
        let q = quantize_position(Vec3::new(x, y, z));
        prop_assert!(q.x < 4096 && q.z < 4096);
        prop_assert!(matches!(q.y, QuantY::Fixed(_)));
        let p = dequantize_position(&q);
        let tol = 1.0 / 128.0 + 1e-9;
        prop_assert!((p.x - x).abs() <= tol, "x {} -> {}", x, p.x);
        prop_assert!((p.y - y).abs() <= tol, "y {} -> {}", y, p.y);
        prop_assert!((p.z - z).abs() <= tol, "z {} -> {}", z, p.z);
    }

    #[test]
    fn orientation_error_under_quarter_degree(
        x in -180.0f64..180.0,
        y in -180.0f64..180.0,
        z in -180.0f64..180.0,
    ) {
        let o = Vec3::new(x, y, z);
        let back = dequantize_orientation(quantize_orientation(o));
        let angle = rotation_angle(o, back).to_degrees();
        prop_assert!(angle < 0.25, "{:?} -> {:?}: {} degrees", o, back, angle);
    }

    #[test]
    fn velocity_round_trip_within_1_16(
        x in -255.875f64..=255.875,
        y in -255.875f64..=255.875,
        z in -255.875f64..=255.875,
    ) {
        let v = dequantize_velocity(quantize_velocity(Vec3::new(x, y, z)));
        prop_assert!((v.x - x).abs() <= 1.0 / 16.0 + 1e-9);
        prop_assert!((v.y - y).abs() <= 1.0 / 16.0 + 1e-9);
        prop_assert!((v.z - z).abs() <= 1.0 / 16.0 + 1e-9);
    }

    #[test]
    fn snapshot_and_input_packets_round_trip(s in snapshot_packet(), i in input_packet(), u in unreliable()) {
        let bytes = encode_snapshot(&s).unwrap();
        prop_assert!(bytes.len() <= MAX_DATAGRAM);
        prop_assert_eq!(datagram_kind(&bytes).unwrap(), DatagramKind::Snapshot);
        prop_assert_eq!(decode_snapshot(&bytes).unwrap(), s);
        let bytes = encode_input(&i).unwrap();
        prop_assert_eq!(datagram_kind(&bytes).unwrap(), DatagramKind::Input);
        prop_assert_eq!(decode_input(&bytes).unwrap(), i);
        if let Ok(bytes) = encode_unreliable_remote(&u) {
            prop_assert_eq!(datagram_kind(&bytes).unwrap(), DatagramKind::UnreliableRemote);
            prop_assert_eq!(decode_unreliable_remote(&bytes).unwrap(), u);
        }
    }

    #[test]
    fn decoders_reject_every_truncation(
        c in client_msg(),
        s in server_msg(),
        snap in snapshot_packet(),
        input in input_packet(),
        u in unreliable(),
    ) {
        let frame = encode_client(&c).unwrap();
        for n in 0..frame.len() {
            prop_assert!(decode_client(&frame[..n]).is_err(), "client prefix {}", n);
        }
        let frame = encode_server(&s).unwrap();
        for n in 0..frame.len() {
            prop_assert!(decode_server(&frame[..n]).is_err(), "server prefix {}", n);
        }
        let d = encode_snapshot(&snap).unwrap();
        for n in 0..d.len() {
            prop_assert!(decode_snapshot(&d[..n]).is_err(), "snapshot prefix {}", n);
        }
        let d = encode_input(&input).unwrap();
        for n in 0..d.len() {
            prop_assert!(decode_input(&d[..n]).is_err(), "input prefix {}", n);
        }
        if let Ok(d) = encode_unreliable_remote(&u) {
            for n in 0..d.len() {
                prop_assert!(decode_unreliable_remote(&d[..n]).is_err(), "remote prefix {}", n);
            }
        }
    }

    #[test]
    fn random_bytes_never_panic_any_decoder(bytes in proptest::collection::vec(any::<u8>(), 0..1500)) {
        run_all_decoders(&bytes);
    }

    #[test]
    fn random_bodies_behind_valid_headers_never_panic(
        kind in 0u8..14,
        seq in any::<u16>(),
        body in proptest::collection::vec(any::<u8>(), 0..1200),
    ) {
        let mut frame = vec![kind, 0];
        write_u64(&mut frame, body.len() as u64);
        frame.extend_from_slice(&body);
        let _ = decode_client(&frame);
        let _ = decode_server(&frame);
        let mut datagram = vec![kind & 0x0f, seq as u8, (seq >> 8) as u8];
        datagram.extend_from_slice(&body[..body.len().min(MAX_DATAGRAM - 3)]);
        run_all_decoders(&datagram);
        let _ = decompress_join(&body);
    }

    #[test]
    fn mutated_valid_frames_never_panic(
        c in client_msg(),
        s in server_msg(),
        snap in snapshot_packet(),
        input in input_packet(),
        flips in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..6),
    ) {
        let datagrams = [encode_snapshot(&snap).unwrap(), encode_input(&input).unwrap()];
        for mut bytes in [encode_client(&c).unwrap(), encode_server(&s).unwrap()]
            .into_iter()
            .chain(datagrams)
        {
            for (at, v) in &flips {
                let i = at.index(bytes.len());
                bytes[i] ^= v;
            }
            run_all_decoders(&bytes);
        }
    }
}

#[test]
fn unknown_kind_is_skipped() {
    let mut frame = encode_server(&ServerMsg::Kick {
        reason: "bye".into(),
    })
    .unwrap();
    frame[0] = 200;
    assert_eq!(decode_server(&frame).unwrap(), Decoded::UnknownKind(200));
    let mut frame = encode_client(&ClientMsg::Bye).unwrap();
    frame[0] = 0;
    assert_eq!(decode_client(&frame).unwrap(), Decoded::UnknownKind(0));
    // An unknown kind still has to be framed correctly.
    assert!(decode_client(&frame[..2]).is_err());
    let mut long = frame.clone();
    long.push(7);
    assert!(decode_client(&long).is_err());
}

#[test]
fn values_convert_both_ways() {
    let mut world = World::new_empty();
    let part = world.create("Part", None).unwrap();
    let hidden = world.create("Part", None).unwrap();
    let net = |id| (id == part).then_some(NetId(77));
    let local = |n: NetId| (n == NetId(77)).then_some(part);
    let cases = [
        (Value::Nil, ValueType::Nil),
        (Value::Bool(true), ValueType::Bool),
        (Value::Number(-12.5), ValueType::Number),
        (Value::String("héllo".into()), ValueType::String),
        (
            Value::Vector3(Vec3::new(1.0, -2.0, 3.5)),
            ValueType::Vector3,
        ),
        (
            Value::Color3(Color3::new(255.0, 0.0, 12.0)),
            ValueType::Color3,
        ),
        (
            Value::UDim2(UDim2::new(0.5, 10.0, 1.0, -4.0)),
            ValueType::UDim2,
        ),
        (Value::Ref(Some(part)), ValueType::Ref),
        (Value::Ref(None), ValueType::Ref),
    ];
    // Every ValueType is covered (the match fails to compile when one is added).
    for (_, ty) in &cases {
        match ty {
            ValueType::Nil
            | ValueType::Bool
            | ValueType::Number
            | ValueType::String
            | ValueType::Vector3
            | ValueType::Color3
            | ValueType::UDim2
            | ValueType::Ref => {}
            // Rebuilt from a quaternion, so it is checked to within rounding below.
            ValueType::CFrame => {}
        }
    }
    let cframes = [
        CFrame::default(),
        CFrame::from_position_orientation(
            Vec3::new(10.0, -3.5, 1e5),
            Vec3::new(30.0, -120.0, 75.0),
        ),
        CFrame::from_position_orientation(Vec3::new(0.0, 0.0, 0.0), Vec3::new(90.0, 180.0, -90.0)),
        CFrame::from_euler_yxz(3.0, -2.0, 1.0),
    ];
    for cf in cframes {
        let wire = NetValue::from_value(&Value::CFrame(Box::new(cf)), net);
        let Some(Value::CFrame(back)) = wire.to_value(ValueType::CFrame, local) else {
            panic!("{cf:?} did not convert back");
        };
        assert_eq!(back.position, cf.position);
        for (a, b) in back.components().iter().zip(cf.components()) {
            assert!((a - b).abs() < 1e-12, "{cf:?} -> {back:?}");
        }
    }
    // A hostile CFrame: non-finite or a zero quaternion is refused.
    let bad = |position: Vec3, rotation: [f64; 4]| {
        NetValue::CFrame(Box::new(NetCFrame { position, rotation }))
            .to_value(ValueType::CFrame, local)
    };
    assert_eq!(
        bad(Vec3::new(f64::NAN, 0.0, 0.0), [0.0, 0.0, 0.0, 1.0]),
        None
    );
    assert_eq!(bad(Vec3::default(), [0.0, 0.0, 0.0, 0.0]), None);
    assert_eq!(bad(Vec3::default(), [f64::INFINITY, 0.0, 0.0, 1.0]), None);
    for (value, ty) in cases {
        let wire = NetValue::from_value(&value, net);
        assert_eq!(wire.to_value(ty, local), Some(value.clone()), "{value:?}");
    }
    assert_eq!(
        NetValue::from_value(&Value::Ref(Some(part)), net),
        NetValue::Ref(Some(NetId(77)))
    );
    // A Ref the receiver cannot see travels as Ref(None) and arrives as a nil Ref.
    let wire = NetValue::from_value(&Value::Ref(Some(hidden)), net);
    assert_eq!(wire, NetValue::Ref(None));
    assert_eq!(wire.to_value(ValueType::Ref, local), Some(Value::Ref(None)));
    assert_eq!(
        NetValue::Ref(Some(NetId(5))).to_value(ValueType::Ref, local),
        Some(Value::Ref(None))
    );
    // Wrong type, non-finite numbers and invalid UTF-8 are refused.
    assert_eq!(
        NetValue::Bool(true).to_value(ValueType::Number, local),
        None
    );
    assert_eq!(
        NetValue::Number(f64::NAN).to_value(ValueType::Number, local),
        None
    );
    assert_eq!(
        NetValue::Vector3(Vec3::new(0.0, f64::INFINITY, 0.0)).to_value(ValueType::Vector3, local),
        None
    );
    assert_eq!(
        NetValue::Str(vec![0xff, 0xfe]).to_value(ValueType::String, local),
        None
    );
    assert_eq!(
        NetValue::Table(vec![]).to_value(ValueType::Nil, local),
        None
    );
}

#[test]
fn service_net_ids_are_fixed() {
    let expected = [
        ("Workspace", 1),
        ("Players", 2),
        ("ReplicatedStorage", 3),
        ("ServerScriptService", 4),
        ("StarterPlayer", 5),
        ("StarterGui", 6),
        ("Lighting", 7),
        ("RunService", 8),
        ("ServerStorage", 9),
        ("StarterPlayerScripts", 10),
    ];
    for (class, id) in expected {
        assert_eq!(service_net_id(class), Some(NetId(id)), "{class}");
        assert!(id < FIRST_DYNAMIC_NET_ID);
    }
    assert_eq!(service_net_id("Part"), None);
    assert_eq!(FIRST_DYNAMIC_NET_ID, 64);
}

#[test]
fn oversize_frames_and_datagrams_are_refused_before_decoding() {
    let big = vec![0u8; 70 * 1024];
    assert_eq!(decode_client(&big), Err(WireError::TooLarge));
    let mut frame = vec![2u8, 0];
    write_u64(&mut frame, (MAX_CLIENT_FRAME + 1) as u64);
    assert!(decode_client(&frame).is_err());
    assert_eq!(
        decode_snapshot(&vec![0u8; MAX_DATAGRAM + 1]),
        Err(WireError::TooLarge)
    );
    assert_eq!(
        decode_input(&vec![0u8; MAX_DATAGRAM + 1]),
        Err(WireError::TooLarge)
    );
    // The encoder refuses what the peer would refuse.
    let huge = ClientMsg::FireServer {
        remote: NetId(70),
        args: vec![NetValue::Str(vec![b'a'; MAX_CLIENT_FRAME])],
    };
    assert_eq!(encode_client(&huge), Err(WireError::TooLarge));
}

#[test]
fn hostile_lengths_do_not_allocate() {
    // FireServer whose args Vec claims u64::MAX - 1 elements, then ends.
    let mut body = Vec::new();
    write_u64(&mut body, 70); // remote
    write_u64(&mut body, u64::MAX >> 1); // args length
    let mut frame = vec![2u8, 0];
    write_u64(&mut frame, body.len() as u64);
    frame.extend_from_slice(&body);
    assert!(decode_client(&frame).is_err());
    // A string claiming 4 GiB.
    let mut body = Vec::new();
    write_u64(&mut body, u32::MAX as u64);
    let mut frame = vec![3u8, 0];
    write_u64(&mut frame, body.len() as u64);
    frame.extend_from_slice(&body);
    assert!(decode_server(&frame).is_err());
}

#[test]
fn deeply_nested_tables_are_refused_not_a_stack_overflow() {
    // FireServer { remote: 70, args: [Table([(Table([(Table(..., Nil)]), Nil)])] } nested 20,000 deep (about 60 KB).
    let depth = 20_000;
    let mut body = Vec::new();
    write_u64(&mut body, 70);
    write_u64(&mut body, 1);
    for _ in 0..depth {
        body.push(9); // Table
        body.push(1); // one pair
    }
    // The innermost key, then each pair's value: all Nil.
    body.extend(std::iter::repeat_n(0u8, depth + 1));
    let mut frame = vec![2u8, 0];
    write_u64(&mut frame, body.len() as u64);
    frame.extend_from_slice(&body);
    assert!(frame.len() <= MAX_CLIENT_FRAME);
    assert!(decode_client(&frame).is_err());

    // The encoder refuses a table the decoder would refuse.
    let mut v = NetValue::Nil;
    for _ in 0..=MAX_TABLE_DEPTH {
        v = NetValue::Table(vec![(v, NetValue::Nil)]);
    }
    let msg = ClientMsg::FireServer {
        remote: NetId(70),
        args: vec![v],
    };
    assert!(encode_client(&msg).is_err());
    // One level less is fine.
    let mut v = NetValue::Nil;
    for _ in 0..MAX_TABLE_DEPTH {
        v = NetValue::Table(vec![(v, NetValue::Nil)]);
    }
    let msg = ClientMsg::FireServer {
        remote: NetId(70),
        args: vec![v],
    };
    let bytes = encode_client(&msg).unwrap();
    assert_eq!(
        decode_client(&bytes).unwrap(),
        Decoded::Msg { source: 0, msg }
    );
}

#[test]
fn join_snapshot_decompression_is_capped() {
    // 17 MiB of zeros compresses to a few hundred bytes; it must be refused, not inflated.
    let bomb = zstd::encode_all(&vec![0u8; 17 << 20][..], 3).unwrap();
    assert!(bomb.len() < 4096);
    assert_eq!(decompress_join(&bomb), Err(WireError::TooLarge));
    assert!(decompress_join(b"not zstd at all").is_err());
}

#[test]
fn quantizers_survive_nan_and_infinity() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300, -1e300] {
        let v = Vec3::new(bad, bad, bad);
        let p = dequantize_position(&quantize_position(v));
        assert!(
            p.x.is_finite() && p.y.is_finite() && p.z.is_finite(),
            "{bad}"
        );
        let o = dequantize_orientation(quantize_orientation(v));
        assert!(
            o.x.is_finite() && o.y.is_finite() && o.z.is_finite(),
            "{bad}"
        );
        let q = quantize_velocity(v);
        assert!(q.iter().all(|c| (-2047..=2047).contains(c)), "{bad}");
    }
    assert_eq!(quantize_velocity(Vec3::new(f64::NAN, 0.0, 0.0)), [0, 0, 0]);
    assert_eq!(
        dequantize_velocity(quantize_velocity(Vec3::new(1000.0, -1000.0, 0.0))).x,
        255.875
    );
    // Out-of-range Y falls back to a raw float, still exact enough.
    let q = quantize_position(Vec3::new(0.0, 50_000.25, 0.0));
    assert_eq!(q.y, QuantY::Raw(50_000.25));
    // Every 32-bit orientation code dequantizes to finite angles.
    for code in [
        0u32,
        u32::MAX,
        0x4000_0000,
        0x8000_0000,
        0xC000_0000,
        0x3FFF_FFFF,
    ] {
        let o = dequantize_orientation(code);
        assert!(
            o.x.is_finite() && o.y.is_finite() && o.z.is_finite(),
            "{code:#x}"
        );
    }
}

#[test]
fn input_packets_must_hold_1_to_8_consecutive_ticks() {
    let cmd = |tick| InputCmd {
        tick,
        move_x: 0,
        move_z: 0,
        jump: false,
        run: false,
        yaw: 0,
        actions: 0,
    };
    let packet = |cmds| InputPacket {
        seq: 1,
        ack: 0,
        ack_bits: 0,
        cmds,
    };
    assert!(encode_input(&packet(vec![])).is_err());
    assert!(encode_input(&packet((0..9).map(cmd).collect())).is_err());
    assert!(encode_input(&packet(vec![cmd(4), cmd(6)])).is_err());
    assert!(encode_input(&packet(vec![cmd(u32::MAX - 1), cmd(u32::MAX)])).is_ok());
}

#[test]
fn non_finite_own_state_is_refused() {
    let own = OwnState {
        position: [0.0, f32::NAN, 0.0],
        vertical: 0.0,
        inherited: [0.0; 3],
        state: 0,
        ground: None,
        yaw: 0.0,
        dead_time: 0.0,
        velocity: [0.0; 3],
        relative: [0.0; 3],
    };
    let packet = SnapshotPacket {
        seq: 0,
        source: 0,
        server_tick: 0,
        last_input_tick: 0,
        buffer_hint: 0,
        own: Some(own),
        entities: vec![],
    };
    assert!(encode_snapshot(&packet).is_err());
}

#[test]
fn hello_debug_redacts_the_ticket() {
    let hello = ClientMsg::Hello {
        ticket: "secret-ticket-bytes".into(),
        protocol_minor: PROTOCOL_MINOR,
        engine_version: "0.1.1".into(),
        registry_hash: [7; 32],
    };
    let text = format!("{hello:?}");
    assert!(!text.contains("secret-ticket-bytes"), "{text}");
    assert!(text.contains("<redacted>") && text.contains("0.1.1"));
}

/// Worst orientation error over quaternions near the corner where all four components are equal (plain rounding of
/// the three kept components reaches about 0.27 degrees there).
#[test]
fn orientation_error_under_quarter_degree_near_equal_components() {
    let check = |q: [f64; 4]| {
        let o = CFrame::from_quaternion(Vec3::default(), q).orientation_degrees();
        rotation_angle(o, dequantize_orientation(quantize_orientation(o))).to_degrees()
    };
    let corner = check([0.499053, 0.499053, 0.499053, 0.502831]);
    assert!(corner < 0.25, "corner: {corner} degrees");
    let mut worst: f64 = 0.0;
    let steps = 16;
    for signs in 0..8u32 {
        let s = |k: u32| if (signs >> k) & 1 == 1 { -1.0 } else { 1.0 };
        for i in -steps..=steps {
            for j in -steps..=steps {
                for k in -steps..=steps {
                    let d = |n: i32| 0.5 + f64::from(n) * 0.0005;
                    worst = worst.max(check([s(0) * d(i), s(1) * d(j), s(2) * d(k), 0.5]));
                }
            }
        }
    }
    assert!(worst < 0.25, "worst near the corner: {worst} degrees");
    println!("worst near the corner: {worst:.4} degrees, corner case {corner:.4}");
}
