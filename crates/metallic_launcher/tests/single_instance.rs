//! Tests for the hand-over message format. The pipe itself is tested in `platform::windows`.

use metallic_launcher::{HandoffMessage, MAX_HANDOFF_BYTES, decode, encode};

#[test]
fn handoff_codec_round_trips() {
    for m in [
        HandoffMessage::Focus,
        HandoffMessage::Link("metallic://install".into()),
    ] {
        assert_eq!(decode(&encode(&m)).unwrap(), m);
    }
}

#[test]
fn encoded_layout_is_length_tag_text() {
    assert_eq!(encode(&HandoffMessage::Focus), b"\x01\x00\x00\x00F");
    assert_eq!(
        encode(&HandoffMessage::Link("ab".into())),
        b"\x03\x00\x00\x00Lab"
    );
}

#[test]
fn garbage_handoff_rejected() {
    for b in [
        &b""[..],
        b"\x05\x00\x00\x00Lab",
        b"\xff\xff\xff\xffL",
        b"\x02\x00\x00\x00X\x00",
        b"\x00\x00\x00\x00",
        b"\x02\x00\x00\x00F\x00",
        b"\x03\x00\x00\x00L\xff\xfe",
        b"\x01\x00",
    ] {
        assert!(decode(b).is_err(), "{b:?}");
    }
}

#[test]
fn oversized_handoff_rejected() {
    // The length field says 4097; the body is not even present, and it is refused anyway.
    let header = ((MAX_HANDOFF_BYTES + 1) as u32).to_le_bytes();
    assert!(decode(&header).is_err());
    assert!(metallic_launcher::single_instance::body_length(header).is_err());
    // The limit itself is fine.
    let mut frame = (MAX_HANDOFF_BYTES as u32).to_le_bytes().to_vec();
    frame.push(b'L');
    frame.extend(std::iter::repeat_n(b'a', MAX_HANDOFF_BYTES - 1));
    assert!(decode(&frame).is_ok());
}
