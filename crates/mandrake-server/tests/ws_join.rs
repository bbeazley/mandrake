mod support;

use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use mandrake_net::endpoint::{Endpoint, Event, HostOptions, Joiner};
use mandrake_net::lobby::BarrierKind;
use mandrake_server::proxy::{self, ProxyConfig};
use mandrake_server::ws::{self, Decoder, Frame};

struct Web {
    ws: TcpStream,
    decoder: Decoder,
    joiner: Joiner,
    closed: Option<Option<u16>>,
}

impl Web {
    fn pump(&mut self, now_ms: u64) -> Vec<Event> {
        let out = self.joiner.take_outgoing();
        if !out.is_empty() {
            let (a, b) = out.split_at(out.len() / 2);
            for part in [a, b] {
                self.ws
                    .write_all(&ws::encode(ws::OP_BINARY, part, Some([1, 2, 3, 4])))
                    .unwrap();
            }
        }
        let mut buf = [0u8; 16384];
        loop {
            match self.ws.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => self.decoder.push(&buf[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => panic!("{e}"),
            }
        }
        while let Some(f) = self.decoder.next_frame().unwrap() {
            match f {
                Frame::Data(d) => self.joiner.receive(&d),
                Frame::Close(c) => self.closed = Some(c),
                Frame::Ping(_) | Frame::Pong => {}
            }
        }
        self.joiner.poll(now_ms)
    }
}

#[test]
fn a_joiner_over_a_websocket_joins_and_plays_through_the_proxy() {
    let description = mandrake_session::net::host_description(
        &mandrake_session::Text(b"Host".to_vec()),
        &mandrake_session::Text(Vec::new()),
        0,
    );
    let options = HostOptions {
        player: support::request(b"Host", b"Team"),
        password: Vec::new(),
        description,
    };
    let mut host = Endpoint::host_at((Ipv4Addr::LOCALHOST, 0).into(), options, 0).unwrap();
    let target = host.listen_addr().unwrap();
    let config = ProxyConfig::parse(&format!("target = \"{target}\"")).unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let at = listener.local_addr().unwrap();
    std::thread::spawn(move || proxy::run(config, listener));

    let (ws, early) = proxy::connect_ws(&at.to_string(), "/myth2-ws").unwrap();
    ws.set_nonblocking(true).unwrap();
    let mut web = Web {
        ws,
        decoder: Decoder::new(false),
        joiner: Joiner::new(support::request(b"Web", b"Team"), 0, 0),
        closed: None,
    };
    web.decoder.push(&early);
    let clock = Instant::now();
    let step = |host: &mut Endpoint, web: &mut Web, until: &dyn Fn(&Event) -> bool| {
        for _ in 0..1000 {
            let t = clock.elapsed().as_millis() as u64;
            host.poll(t);
            if web.pump(t).iter().any(until) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    };
    assert!(step(&mut host, &mut web, &|e| matches!(
        e,
        Event::Description(_)
    )));
    assert_eq!(web.joiner.local_id(), Some(1));
    assert_ne!(
        web.joiner.host_udp_port(),
        None,
        "the host's greeting came through"
    );
    assert!(web.joiner.set_ready(true));
    let h = host.as_host_mut().unwrap();
    h.start(42);
    h.begin_barrier(BarrierKind::Start);
    assert!(step(&mut host, &mut web, &|e| matches!(
        e,
        Event::BeginSync { .. }
    )));
    assert!(web.joiner.loaded());
    assert!(host.joiner_mut().loaded());
    assert!(step(&mut host, &mut web, &|e| matches!(
        e,
        Event::AllSynced { .. }
    )));
    assert!(web.joiner.send_command(16, vec![1, 2, 3]));
    assert!(step(&mut host, &mut web, &|e| matches!(
        e,
        Event::Commands { commands, .. } if commands.iter().any(|c| c.kind == 16 && c.player == 1)
    )));
    host.leave();
    let t0 = Instant::now();
    while web.closed.is_none() && t0.elapsed() < Duration::from_secs(5) {
        web.pump(clock.elapsed().as_millis() as u64);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(web.closed, Some(Some(ws::CLOSE_NORMAL)));
}
