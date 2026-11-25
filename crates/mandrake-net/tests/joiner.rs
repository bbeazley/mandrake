use mandrake_net::client::DEFAULT_FORMATION;
use mandrake_net::endpoint::{Endpoint, Event, Joiner};
use mandrake_net::player_data::{JoinRequest, PlayerData};
use mandrake_net::topology::JoinState;
use mandrake_net::wire::*;

fn request(name: &[u8]) -> JoinRequest {
    JoinRequest {
        name: name.to_vec(),
        password: Vec::new(),
        player: PlayerData::new(name, b"Team", [0; 8]),
    }
}

fn host(to: u8, body: Body) -> Vec<u8> {
    Message::new(to, 0xfe, body).encode().unwrap()
}

fn messages_of(bytes: &[u8]) -> Vec<Message> {
    let mut s = Splitter::without_greeting();
    s.push(bytes);
    let mut out = Vec::new();
    while let Some(Frame::Message(m)) = s.next_frame().unwrap() {
        out.push(Message::decode(&m).unwrap());
    }
    out
}

fn joined() -> Joiner {
    let mut j = Joiner::new(request(b"Guest"), 1234, 0);
    j.take_outgoing();
    j.receive(&encode_greeting(4321));
    for (id, flags, name) in [(0u16, 0u16, &b"Host"[..]), (1, 1, b"Guest")] {
        let mut p = PlayerData::new(name, b"Team", [0; 8]);
        p.net_id = id as i32;
        j.receive(&host(
            1,
            Body::NewPlayer {
                id,
                flags,
                name: name.to_vec(),
                player_data: p.to_bytes().to_vec(),
            },
        ));
    }
    let events = j.poll(10);
    assert!(events.contains(&Event::Greeting(4321)));
    assert!(events.contains(&Event::Joined { id: 1 }));
    assert_eq!(j.local_id(), Some(1));
    assert_eq!(j.host_udp_port(), Some(4321));
    j
}

#[test]
fn no_answer_in_5_s_times_the_join_out() {
    let mut j = Joiner::new(request(b"Guest"), 1, 1000);
    assert!(j.poll(5999).is_empty());
    assert_eq!(j.poll(6000), [Event::JoinTimedOut]);
    assert_eq!(j.join_state(), JoinState::Failed);
    assert!(j.poll(9000).is_empty());
}

#[test]
fn a_denial_carries_the_reason_and_no_id() {
    let mut j = Joiner::new(request(b"Guest"), 1, 0);
    j.receive(&encode_greeting(2));
    j.receive(&host(
        0,
        Body::JoinDenied {
            reason: b"The game is full".to_vec(),
            error: None,
        },
    ));
    let events = j.poll(100);
    assert_eq!(
        events.last(),
        Some(&Event::Denied {
            reason: b"The game is full".to_vec()
        })
    );
    assert_eq!(j.local_id(), None);
    assert!(!j.set_ready(true));
    assert!(!j.send_command(16, vec![]));
    assert!(j.poll(10_000).is_empty(), "no timeout after a denial");
}

#[test]
fn a_ping_gets_a_pong_echoing_its_ticks() {
    let mut j = joined();
    j.receive(&host(
        1,
        Body::Ping {
            sdl_ticks: 0x0001_33d4,
            latencies: vec![0, 0x31],
        },
    ));
    let ids: Vec<u16> = j.players().iter().map(|p| p.id).collect();
    let expected: Vec<(u16, u16)> = ids.into_iter().zip([0, 0x31]).collect();
    assert_eq!(j.poll(20), [Event::Latencies(expected)]);
    let out = j.take_outgoing();
    assert_eq!(
        out,
        [
            0x00, 0x09, 0x00, 0x0c, 0xfe, 0x01, 0, 0, 0xd4, 0x33, 0x01, 0x00
        ]
    );
}

#[test]
fn ready_sends_our_record_with_our_id() {
    let mut j = joined();
    assert!(j.set_ready(true));
    let m = &messages_of(&j.take_outgoing())[0];
    assert_eq!((m.to, m.from), (0xfe, 1));
    let Body::PlayerDataChanged { id, player_data } = &m.body else {
        panic!("{m:?}")
    };
    let p = PlayerData::read(player_data).unwrap();
    assert_eq!((*id, p.net_id, p.ready), (1, 1, 1));
}

#[test]
fn the_horizon_follows_stamped_ticks_and_nulls_are_not_kept() {
    let mut j = joined();
    let cmd = |kind, player, tick| Command {
        kind,
        player,
        tick,
        payload: if kind == 0 { vec![] } else { vec![1, 2] },
    };
    for c in [cmd(0, 0xff, 3), cmd(16, 0, 4), cmd(0, 0xff, 8)] {
        j.receive(&host(0xff, Body::Command(c)));
    }
    let events = j.poll(30);
    assert_eq!(
        events,
        [Event::Commands {
            commands: vec![cmd(16, 0, 4)],
            horizon: 8
        }]
    );
    assert_eq!(j.horizon(), 8);
    j.receive(&host(0xff, Body::Command(cmd(0, 0xff, 5))));
    assert!(j.poll(40).is_empty());
    assert_eq!(j.horizon(), 8);
    j.new_map();
    assert_eq!(j.horizon(), 0);
}

#[test]
fn a_command_goes_up_unstamped_to_the_server() {
    let mut j = joined();
    assert!(j.send_command(16, DEFAULT_FORMATION.to_vec()));
    let out = j.take_outgoing();
    let mut expect = vec![0x00, 0x01, 0x00, 0x38, 0xfe, 0x01, 0, 0];
    expect.extend_from_slice(&[0x00, 0x30, 0x10, 0xff, 0, 0, 0, 0]);
    expect.extend_from_slice(&DEFAULT_FORMATION);
    assert_eq!(out, expect);
}

#[test]
fn a_barrier_is_answered_once_and_ends_when_every_player_is_listed() {
    let mut j = joined();
    assert!(!j.loaded(), "no barrier yet");
    j.receive(&host(0xff, Body::BeginSyncData));
    assert_eq!(j.poll(50), [Event::BeginSync { barrier: 1 }]);
    assert!(j.loaded());
    assert!(!j.loaded(), "once");
    assert_eq!(j.take_outgoing(), [0, 3, 0, 0x0c, 0, 1, 0, 0, 0, 1, 0, 1]);
    j.receive(&host(0xff, Body::PlayerSyncData { id: 0, value: 0 }));
    assert!(j.poll(60).is_empty());
    j.receive(&host(0xff, Body::PlayerSyncData { id: 1, value: 0 }));
    assert_eq!(j.poll(70), [Event::AllSynced { barrier: 1 }]);
    j.receive(&host(0xff, Body::BeginSyncData));
    assert_eq!(j.poll(80), [Event::BeginSync { barrier: 2 }]);
}

#[test]
fn a_lost_host_is_an_event() {
    let mut j = joined();
    assert_eq!(j.connection_lost(), Event::HostLost);
    assert_eq!(j.join_state(), JoinState::Failed);
}

#[test]
fn an_endpoint_joins_a_fake_host_on_loopback() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let mut ep = Endpoint::join(addr, request(b"Guest"), 0).unwrap();
    let (mut h, _) = listener.accept().unwrap();
    let mut first = vec![0u8; 2];
    h.read_exact(&mut first).unwrap();
    let mut header = [0u8; 8];
    h.read_exact(&mut header).unwrap();
    assert_eq!(&header[..2], [0x80, 0x00]);
    let len = u16::from_be_bytes([header[2], header[3]]) as usize;
    let mut rest = vec![0u8; len - 8];
    h.read_exact(&mut rest).unwrap();
    let mut p = PlayerData::new(b"Guest", b"Team", [0; 8]);
    p.net_id = 0;
    h.write_all(&encode_greeting(9)).unwrap();
    h.write_all(&host(
        1,
        Body::NewPlayer {
            id: 1,
            flags: 1,
            name: b"Guest".to_vec(),
            player_data: p.to_bytes().to_vec(),
        },
    ))
    .unwrap();
    let mut joined = false;
    for t in 0..400 {
        if ep.poll(t).contains(&Event::Joined { id: 1 }) {
            joined = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(joined);
    drop(h);
    let mut lost = false;
    for t in 400..800 {
        if ep.poll(t).contains(&Event::HostLost) {
            lost = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(lost);
}

#[test]
fn a_denial_after_we_joined_is_ignored() {
    let mut j = joined();
    j.receive(&host(
        1,
        Body::JoinDenied {
            reason: b"Late".to_vec(),
            error: None,
        },
    ));
    let events = j.poll(100);
    assert!(!events.iter().any(|e| matches!(e, Event::Denied { .. })));
    assert_eq!(j.local_id(), Some(1));
    assert!(j.set_ready(true));
}

#[test]
fn a_new_player_whose_id_is_past_the_last_net_id_is_malformed() {
    let mut j = Joiner::new(request(b"Guest"), 1, 0);
    j.receive(&encode_greeting(2));
    let p = PlayerData::new(b"Guest", b"Team", [0; 8]);
    j.receive(&host(
        0,
        Body::NewPlayer {
            id: 0x100,
            flags: 1,
            name: b"Guest".to_vec(),
            player_data: p.to_bytes().to_vec(),
        },
    ));
    let events = j.poll(100);
    assert!(events.contains(&Event::Malformed(kind::NEW_PLAYER)));
    assert_eq!(j.local_id(), None, "not the host's id 0");
    assert!(j.players().is_empty());
}

#[test]
fn a_joiner_closes_its_socket_on_a_stream_it_cannot_follow() {
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let mut e = Endpoint::join(addr, request(b"Guest"), 0).unwrap();
    let (mut peer, _) = listener.accept().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut bad = encode_greeting(2).to_vec();
    bad.extend_from_slice(&[0, 0x0d, 0, 3, 1, 0xfe, 0, 0]);
    peer.write_all(&bad).unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    let mut seen = Vec::new();
    while !seen.iter().any(|e| matches!(e, Event::StreamError(_))) && Instant::now() < until {
        e.wait(Duration::from_millis(20));
        seen.extend(e.poll(10));
    }
    assert_eq!(
        seen.iter().filter(|e| **e == Event::HostLost).count(),
        1,
        "{seen:?}"
    );
    let mut rest = Vec::new();
    peer.read_to_end(&mut rest).expect("EOF, not a timeout");
    assert!(e.poll(20).is_empty());
}
