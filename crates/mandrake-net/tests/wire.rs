//! The wire's small parts against bytes from the captures (docs/research/network/).

use mandrake_net::wire::*;

fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

#[test]
fn the_greeting_is_the_udp_port_big_endian() {
    // NETWORK.md section 1: `83 c2`, the joiner's UDP port 33730.
    assert_eq!(encode_greeting(33730), [0x83, 0xc2]);
    assert_eq!(decode_greeting([0x83, 0xc2]), 33730);
    let mut s = Splitter::with_greeting();
    s.push(&[0x83]);
    assert_eq!(s.next_frame(), Ok(None));
    s.push(&[0xc2, 0x00, 0x04]);
    assert_eq!(s.next_frame(), Ok(Some(Frame::Greeting(33730))));
    assert_eq!(s.next_frame(), Ok(None));
    assert_eq!(s.pending(), 2);
}

/// run1-messages.txt at 11.406 s: the host's lobby chat "hello-from-host".
const HOST_CHAT: &str = "00 00 00 48 ff 00 00 00 48 6f 73 74 00 00 00 00 00 00 00 00 00 00 00 00 \
    00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 59 98 08 f3 2f 4b 00 00 00 00 00 02 ff ff 00 00 \
    75 72 79 79 7c 3a 73 7f 7c 7a 3a 75 7c 80 81 00";

#[test]
fn lobby_chat_text_travels_plus_13() {
    let bytes = hex(HOST_CHAT);
    let m = Message::decode(&bytes).unwrap();
    let Body::Chat(chat) = &m.body else {
        panic!("{m:?}")
    };
    assert_eq!(chat.text, b"hello-from-host");
    assert_eq!(&chat.name[..5], b"Host\0");
    assert_eq!((m.to, m.from), (TO_ALL, 0));

    // Built from the plain text, it gives the captured bytes back.
    let mut name = [0; 32];
    name[..4].copy_from_slice(b"Host");
    let built = Message::new(
        TO_ALL,
        0,
        Body::Chat(Chat {
            name,
            words: chat.words,
            text: b"hello-from-host".to_vec(),
        }),
    );
    assert_eq!(built.encode().unwrap(), bytes);
    // Every text byte is shifted, the terminator is not.
    assert_eq!(bytes[56], b'h' + 13);
    assert_eq!(*bytes.last().unwrap(), 0);
}

#[test]
fn ping_and_pong_carry_the_ticks_unswapped() {
    // run1-messages.txt at 1.278 s and 1.303 s.
    let ping = hex("00 09 00 10 01 00 00 00 cd ff 00 00 00 00 00 32");
    let pong = hex("00 09 00 0c fe 01 00 00 cd ff 00 00");
    assert_eq!(
        Message::decode(&ping).unwrap().body,
        Body::Ping {
            sdl_ticks: 0xffcd,
            latencies: vec![0, 0x32]
        }
    );
    assert_eq!(
        Message::decode(&pong).unwrap().body,
        Body::Pong { sdl_ticks: 0xffcd }
    );
    assert_eq!(
        Message::new(TO_HOST, 1, Body::Pong { sdl_ticks: 0xffcd })
            .encode()
            .unwrap(),
        pong
    );
}

#[test]
fn the_small_bodies_decode_as_the_captures_show() {
    // The start: configuration {flags 0, code 100}, begin-sync, the sync data (run 1, 13.9 s).
    let config = hex("00 02 00 10 ff 00 00 00 00 00 00 64 00 00 00 00");
    assert_eq!(
        Message::decode(&config).unwrap(),
        Message::new(
            TO_ALL,
            0,
            Body::Configuration {
                flags: 0,
                code: 100,
                value: 0
            }
        )
    );
    let begin = hex("00 04 00 08 ff 00 00 00");
    assert_eq!(Message::decode(&begin).unwrap().body, Body::BeginSyncData);
    let sync = hex("00 03 00 0c 00 01 00 00 00 01 00 01");
    assert_eq!(
        Message::decode(&sync).unwrap().body,
        Body::PlayerSyncData { id: 1, value: 1 }
    );
    // A stamped SYNC_FORMATION header (run 1, 14.812 s): type 0x10, player 1, tick 5.
    let cmd = hex("00 01 00 10 01 00 00 00 00 08 10 01 00 00 00 05");
    assert_eq!(
        Message::decode(&cmd).unwrap().body,
        Body::Command(Command {
            kind: 0x10,
            player: 1,
            tick: 5,
            payload: vec![]
        })
    );
}

#[test]
fn the_splitter_refuses_lengths_outside_8_to_0x1000() {
    for bad in [0u16, 4, 7, 0x1001, 0xffff] {
        let mut s = Splitter::without_greeting();
        let mut bytes = vec![0, 4];
        bytes.extend_from_slice(&bad.to_be_bytes());
        bytes.extend_from_slice(&[0xff, 0, 0, 0]);
        s.push(&bytes);
        assert_eq!(s.next_frame(), Err(WireError::BadLength(bad)));
        // The error stays: the engine drops the player.
        assert_eq!(s.next_frame(), Err(WireError::BadLength(bad)));
    }
    let mut s = Splitter::without_greeting();
    let mut big = vec![0, 8, 0x10, 0x00, 0xff, 0xfe, 0, 0];
    big.resize(0x1000, 0xaa);
    s.push(&big[..0xfff]);
    assert_eq!(s.next_frame(), Ok(None));
    s.push(&big[0xfff..]);
    assert_eq!(s.next_frame(), Ok(Some(Frame::Message(big))));
}

#[test]
fn an_unknown_type_does_not_decode() {
    let bytes = hex("00 0e 00 08 ff 00 00 00");
    assert_eq!(Message::decode(&bytes), Err(WireError::UnknownType(0xe)));
    let bytes = hex("80 08 00 08 ff 00 00 00");
    assert_eq!(Message::decode(&bytes), Err(WireError::UnknownType(0x8008)));
}

#[test]
fn a_chat_byte_the_shift_makes_0_ends_the_text() {
    let chat = |text: &[u8]| Chat {
        name: [0; 32],
        words: [0; 8],
        text: text.to_vec(),
    };
    let bytes = Message::new(0, 1, Body::Chat(chat(b"ab\xf3cd")))
        .encode()
        .unwrap();
    assert_eq!(bytes.len(), HEADER_SIZE + 32 + 16 + 3);
    assert_eq!(
        Message::decode(&bytes).unwrap().body,
        Body::Chat(chat(b"ab"))
    );
}

#[test]
fn a_chat_line_with_bytes_after_its_terminator_reads_to_the_terminator() {
    let mut body = vec![0; 32 + 16];
    body.extend([
        b'a' + CHAT_SHIFT,
        b'b' + CHAT_SHIFT,
        0,
        b'c' + CHAT_SHIFT,
        0,
    ]);
    let mut bytes = Header {
        kind: kind::CHAT,
        length: (HEADER_SIZE + body.len()) as u16,
        to: 0,
        from: 1,
        spare: 0,
    }
    .to_bytes()
    .to_vec();
    bytes.extend(body);
    let Body::Chat(c) = Message::decode(&bytes).unwrap().body else {
        panic!("a chat");
    };
    assert_eq!(c.text, b"ab");
}
