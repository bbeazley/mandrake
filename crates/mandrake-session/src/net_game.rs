use std::time::Duration;

use mandrake_net::endpoint::{Endpoint, Event, HostEvent};
use mandrake_net::lobby::BarrierKind;
use mandrake_net::wire::Command;
pub use mandrake_rec::CommandPacket;
use tracing::{error, info, warn};

pub const END_BARRIER_WAIT: Duration = Duration::from_secs(30);
pub const LEAD_LOG_TICKS: u32 = 300;

pub trait NetSim {
    fn current_tick(&self) -> u32;
    fn net_horizon(&self) -> Option<u32>;
    fn queue_incoming_command(&mut self, packet: CommandPacket);
    fn take_outgoing_commands(&mut self) -> Vec<CommandPacket>;
    fn update_head(&mut self, filling: bool);
    fn net_game_speed(&self) -> Option<i16>;
    fn ticks_stopped(&self) -> bool;
    fn net_body_may_run(&mut self) -> bool;
}

pub trait NetPace {
    fn filling(&self) -> bool;
    fn lead(&self) -> i16;
    fn starvations(&self) -> i16;
    fn pend_network_index(&mut self, index: i16);
    fn advance_network(&mut self, polls: u32, paused: bool, tick: u32, horizon: u32) -> FrameStep;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FrameStep {
    pub bodies: u32,
    pub requested: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetFrame {
    left: u32,
    pub step: FrameStep,
}

impl NetFrame {
    pub fn next_body(&mut self, sim: &mut dyn NetSim) -> bool {
        if self.left == 0 || !sim.net_body_may_run() {
            self.left = 0;
            return false;
        }
        self.left -= 1;
        true
    }
}

#[macro_export]
macro_rules! net_sim_adapters {
    ($vis:vis, $sim:ty, $speed:ty, $gate:ty) => {
        $vis struct SimNet<'a>(pub &'a mut $sim);

        impl $crate::net_game::NetSim for SimNet<'_> {
            fn current_tick(&self) -> u32 {
                self.0.current_tick
            }
            fn net_horizon(&self) -> Option<u32> {
                self.0.net_horizon()
            }
            fn queue_incoming_command(&mut self, packet: $crate::net_game::CommandPacket) {
                self.0.queue_incoming_command(packet);
            }
            fn take_outgoing_commands(&mut self) -> Vec<$crate::net_game::CommandPacket> {
                self.0.take_outgoing_commands()
            }
            fn update_head(&mut self, filling: bool) {
                self.0.net_head(filling);
            }
            fn net_game_speed(&self) -> Option<i16> {
                self.0.net_game_speed()
            }
            fn ticks_stopped(&self) -> bool {
                self.0.ticks_stopped()
            }
            fn net_body_may_run(&mut self) -> bool {
                self.0.net_body_may_run()
            }
        }

        $vis struct Pace<'a> {
            pub speed: &'a mut $speed,
            pub gate: &'a mut $gate,
        }

        impl $crate::net_game::NetPace for Pace<'_> {
            fn filling(&self) -> bool {
                self.gate.fill
            }
            fn lead(&self) -> i16 {
                self.gate.lead
            }
            fn starvations(&self) -> i16 {
                self.gate.starvations()
            }
            fn pend_network_index(&mut self, index: i16) {
                self.speed.pend_network_index(index);
            }
            fn advance_network(
                &mut self,
                polls: u32,
                paused: bool,
                tick: u32,
                horizon: u32,
            ) -> $crate::net_game::FrameStep {
                self.speed
                    .advance_network(self.gate, polls, paused, tick, horizon)
                    .map_or($crate::net_game::FrameStep::default(), |s| {
                        $crate::net_game::FrameStep {
                            bodies: s.bodies,
                            requested: s.requested,
                        }
                    })
            }
        }
    };
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NetNews {
    pub host_lost: bool,
    pub started: bool,
    pub ended: bool,
}

pub type Clock = Box<dyn Fn() -> Duration + Send>;

pub struct NetLoop {
    pub endpoint: Endpoint,
    clock: Clock,
    log: &'static str,
    loaded: bool,
    pregame_answered: bool,
    pub started: bool,
    over_at: Option<Duration>,
    end_answered: bool,
    pub ended: bool,
    pub host_lost: bool,
    host_barriers: (bool, bool),
    next_lead_log: u32,
    gate_seen: Option<(bool, i16)>,
    pub sent: usize,
    pub received: usize,
}

impl NetLoop {
    pub fn new(endpoint: Endpoint, clock: Clock, log: &'static str) -> Self {
        Self {
            endpoint,
            clock,
            log,
            loaded: false,
            pregame_answered: false,
            started: false,
            over_at: None,
            end_answered: false,
            ended: false,
            host_lost: false,
            host_barriers: (false, false),
            next_lead_log: 0,
            gate_seen: None,
            sent: 0,
            received: 0,
        }
    }

    pub fn elapsed(&self) -> Duration {
        (self.clock)()
    }

    pub fn now_ms(&self) -> u64 {
        self.elapsed().as_millis() as u64
    }

    pub fn begin_map(&mut self) {
        self.endpoint.joiner_mut().new_map();
        self.loaded = false;
        self.pregame_answered = false;
        self.started = false;
        self.over_at = None;
        self.end_answered = false;
        self.ended = false;
        self.next_lead_log = 0;
        self.gate_seen = None;
        self.host_barriers = (false, false);
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn is_over(&self) -> bool {
        self.over_at.is_some()
    }

    pub fn map_loaded(&mut self) {
        self.loaded = true;
        if !self.host_barriers.0
            && let Some(host) = self.endpoint.as_host_mut()
        {
            self.host_barriers.0 = true;
            host.begin_barrier(BarrierKind::Start);
            info!("{}: host: the pre-game barrier begins", self.log);
        }
        self.answer_barriers();
    }

    pub fn map_over(&mut self) {
        if self.over_at.is_none() {
            info!(
                "{}: the game is over here; the end barrier is answered when it begins",
                self.log
            );
            self.over_at = Some(self.elapsed());
        }
        if !self.host_barriers.1
            && let Some(host) = self.endpoint.as_host_mut()
        {
            self.host_barriers.1 = true;
            host.begin_barrier(BarrierKind::End);
            info!(
                "{}: host: the server stops; the end barrier begins",
                self.log
            );
        }
        self.answer_barriers();
    }

    pub fn end_given_up(&self) -> bool {
        self.host_lost
            || self
                .over_at
                .is_some_and(|at| self.elapsed().saturating_sub(at) > END_BARRIER_WAIT)
    }

    pub fn answer_barriers(&mut self) {
        let j = self.endpoint.joiner_mut();
        if self.loaded && !self.pregame_answered && j.loaded() {
            self.pregame_answered = true;
            info!("{}: loaded; the pre-game barrier answered", self.log);
        }
        if self.over_at.is_some() && self.started && !self.end_answered && j.loaded() {
            self.end_answered = true;
            info!("{}: the end barrier answered", self.log);
        }
        self.endpoint.flush();
    }

    pub fn poll_events(
        &mut self,
        mut sim: Option<&mut dyn NetSim>,
        mut on_command: impl FnMut(&Command),
    ) -> (NetNews, Vec<Event>) {
        let mut news = NetNews::default();
        let now = self.now_ms();
        let events = self.endpoint.poll(now);
        for event in &events {
            match event {
                Event::Commands { commands, .. } => {
                    if let Some(sim) = sim.as_deref_mut() {
                        for c in commands {
                            self.received += 1;
                            on_command(c);
                            sim.queue_incoming_command(CommandPacket::parse(
                                c.tick.max(0) as u32,
                                c.player,
                                c.kind,
                                &c.payload,
                            ));
                        }
                    }
                }
                Event::AllSynced { barrier } => {
                    info!("{}: barrier {barrier} done", self.log);
                    if self.loaded && !self.started {
                        self.started = true;
                        news.started = true;
                    } else if self.end_answered && !self.ended {
                        self.ended = true;
                        news.ended = true;
                    }
                }
                Event::HostLost if !self.host_lost => {
                    self.host_lost = true;
                    news.host_lost = true;
                    error!(
                        "{}: the host is gone at tick {:?}",
                        self.log,
                        sim.as_deref().map(|s| s.current_tick())
                    );
                }
                _ => {}
            }
        }
        (news, events)
    }

    pub fn heartbeat(&mut self, sim: &mut dyn NetSim) {
        let horizon = self.endpoint.joiner().horizon();
        if horizon > 0 && sim.net_horizon().is_some_and(|h| (h as i32) < horizon) {
            sim.queue_incoming_command(CommandPacket::parse(horizon as u32, 0xff, 0, &[]));
        }
    }

    pub fn end_poll(&mut self, sim: Option<&mut dyn NetSim>) {
        if let Some(sim) = sim {
            self.heartbeat(sim);
        }
        self.answer_barriers();
    }

    pub fn send_outgoing(
        &mut self,
        sim: &mut dyn NetSim,
        more: Vec<CommandPacket>,
        mut on_sent: impl FnMut(&CommandPacket),
    ) {
        let mut outgoing = sim.take_outgoing_commands();
        outgoing.extend(more);
        for c in outgoing {
            let bytes = c.to_bytes();
            if self
                .endpoint
                .joiner_mut()
                .send_command(bytes[2], bytes[8..].to_vec())
            {
                self.sent += 1;
                on_sent(&c);
            }
        }
        self.endpoint.flush();
    }

    pub fn frame(&mut self, sim: &mut dyn NetSim, pace: &mut dyn NetPace, polls: u32) -> NetFrame {
        if polls > 0 {
            sim.update_head(pace.filling());
            if let Some(index) = sim.net_game_speed() {
                pace.pend_network_index(index);
            }
        }
        let paused = sim.ticks_stopped();
        let tick = sim.current_tick();
        let horizon = sim.net_horizon().unwrap_or(0);
        let last = *self.gate_seen.get_or_insert((false, pace.lead()));
        let step = pace.advance_network(polls, paused, tick, horizon);
        let now = (pace.filling(), pace.lead());
        if now != last {
            if now.0 && !last.0 {
                info!(
                    "{}: FILL at tick {tick} horizon {horizon} lead {} starvations {}",
                    self.log,
                    now.1,
                    pace.starvations()
                );
            }
            if now.1 != last.1 {
                info!("{}: LEAD raised to {} at tick {tick}", self.log, now.1);
            }
            self.gate_seen = Some(now);
        }
        NetFrame {
            left: step.bodies,
            step,
        }
    }

    pub fn lead_log_due(&mut self, tick: u32) -> bool {
        if tick >= self.next_lead_log {
            self.next_lead_log = (tick / LEAD_LOG_TICKS + 1) * LEAD_LOG_TICKS;
            return true;
        }
        false
    }
}

pub fn log_host_event(prefix: &str, e: &HostEvent, tick: Option<u32>) {
    match e {
        HostEvent::Joined { id, name } => {
            info!(
                "{prefix}player {id} \"{}\" joined",
                String::from_utf8_lossy(name)
            )
        }
        HostEvent::PlayerLeft {
            id,
            joined,
            arrived,
        } => {
            info!("{prefix}player {id} left at tick {tick:?} (joined {joined}, arrived {arrived})")
        }
        HostEvent::Denied { id, denial } => info!("{prefix}player {id} denied: {denial:?}"),
        HostEvent::Arrived { .. } | HostEvent::PlayerChanged { .. } => {}
        HostEvent::Undecoded { .. } | HostEvent::StreamError { .. } => warn!("{prefix}{e:?}"),
        HostEvent::RecordRefused { from, id } => {
            warn!("{prefix}player {from}'s record of player {id} refused at tick {tick:?}")
        }
        HostEvent::NotRelayed { from, to, kind } => warn!(
            "{prefix}player {from}'s message of type {kind:#x} to {to:#x} not passed on at tick \
             {tick:?}"
        ),
        other => info!("{prefix}{other:?}"),
    }
}

#[derive(Debug)]
pub struct PollClock {
    last: Duration,
    accumulator: f32,
}

impl PollClock {
    pub fn new(now: Duration) -> Self {
        Self {
            last: now,
            accumulator: 0.0,
        }
    }

    pub fn polls(&mut self, now: Duration) -> u32 {
        self.accumulator += now.saturating_sub(self.last).as_secs_f32().min(0.25);
        self.last = now;
        let mut polls = 0;
        while self.accumulator >= 1.0 / 30.0 {
            self.accumulator -= 1.0 / 30.0;
            polls += 1;
        }
        polls
    }
}

pub fn machine_ticks(since_epoch: Duration) -> u32 {
    since_epoch.as_millis() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_poll_clock_counts_30_hz_and_at_most_a_quarter_second() {
        let mut c = PollClock::new(Duration::ZERO);
        assert_eq!(c.polls(Duration::from_millis(10)), 0);
        assert_eq!(c.polls(Duration::from_millis(34)), 1);
        assert_eq!(c.polls(Duration::from_millis(2034)), 7);
    }

    #[derive(Debug)]
    struct Silent;

    impl mandrake_net::transport::ByteStream for Silent {
        fn read(&mut self, _: &mut Vec<u8>) -> std::io::Result<()> {
            Ok(())
        }
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            Ok(bytes.len())
        }
        fn close(&mut self) {}
    }

    #[derive(Default)]
    struct Fake {
        calls: Vec<String>,
        tick: u32,
        horizon: u32,
        speed: Option<i16>,
        may_run: u32,
    }

    impl NetSim for Fake {
        fn current_tick(&self) -> u32 {
            self.tick
        }
        fn net_horizon(&self) -> Option<u32> {
            Some(self.horizon)
        }
        fn queue_incoming_command(&mut self, _: CommandPacket) {}
        fn take_outgoing_commands(&mut self) -> Vec<CommandPacket> {
            Vec::new()
        }
        fn update_head(&mut self, filling: bool) {
            self.calls.push(format!("head {filling}"));
        }
        fn net_game_speed(&self) -> Option<i16> {
            self.speed
        }
        fn ticks_stopped(&self) -> bool {
            false
        }
        fn net_body_may_run(&mut self) -> bool {
            self.calls.push("may run".into());
            let ok = self.may_run > 0;
            self.may_run = self.may_run.saturating_sub(1);
            ok
        }
    }

    struct FakePace(Vec<String>, bool, i16, u32);

    impl NetPace for FakePace {
        fn filling(&self) -> bool {
            self.1
        }
        fn lead(&self) -> i16 {
            self.2
        }
        fn starvations(&self) -> i16 {
            0
        }
        fn pend_network_index(&mut self, index: i16) {
            self.0.push(format!("speed {index}"));
        }
        fn advance_network(&mut self, polls: u32, _: bool, tick: u32, horizon: u32) -> FrameStep {
            self.0.push(format!("gate {polls} {tick} {horizon}"));
            FrameStep {
                bodies: self.3,
                requested: polls,
            }
        }
    }

    fn net_loop() -> NetLoop {
        let request = mandrake_net::player_data::JoinRequest {
            name: b"A".to_vec(),
            password: Vec::new(),
            player: mandrake_net::player_data::PlayerData::new(b"A", b"T", [0; 8]),
        };
        let endpoint = Endpoint::join_over(Box::new(Silent), request, 0);
        NetLoop::new(endpoint, Box::new(|| Duration::ZERO), "test")
    }

    #[test]
    fn a_frame_runs_the_head_the_gate_then_its_bodies() {
        let mut net = net_loop();
        let mut sim = Fake {
            tick: 40,
            horizon: 48,
            speed: Some(3),
            may_run: 2,
            ..Fake::default()
        };
        let mut pace = FakePace(Vec::new(), true, 4, 5);
        let mut frame = net.frame(&mut sim, &mut pace, 2);
        assert_eq!(pace.0, ["speed 3", "gate 2 40 48"]);
        assert_eq!(sim.calls, ["head true"]);
        assert_eq!(
            frame.step,
            FrameStep {
                bodies: 5,
                requested: 2
            }
        );
        let mut ran = 0;
        while frame.next_body(&mut sim) {
            ran += 1;
        }
        assert_eq!(ran, 2);
        assert_eq!(sim.calls, ["head true", "may run", "may run", "may run"]);
        assert!(!frame.next_body(&mut sim));

        let (mut sim, mut pace) = (
            Fake {
                speed: Some(1),
                may_run: 9,
                ..Fake::default()
            },
            FakePace(Vec::new(), false, 4, 1),
        );
        let mut frame = net.frame(&mut sim, &mut pace, 0);
        assert_eq!(pace.0, ["gate 0 0 0"]);
        assert!(frame.next_body(&mut sim));
        assert!(!frame.next_body(&mut sim));
        assert_eq!(sim.calls, ["may run"]);
    }

    #[test]
    fn begin_map_requires_pregame_barrier_before_end_barrier_on_map_two() {
        let options = mandrake_net::endpoint::HostOptions {
            player: mandrake_net::player_data::JoinRequest {
                name: b"Host".to_vec(),
                password: Vec::new(),
                player: mandrake_net::player_data::PlayerData::new(b"Host", b"T", [0; 8]),
            },
            password: Vec::new(),
            description: crate::net::host_description(
                &crate::text::Text(b"Host".to_vec()),
                &crate::text::Text::default(),
                0,
            ),
        };
        let endpoint = Endpoint::host_at("127.0.0.1:0".parse().unwrap(), options, 0).unwrap();
        let ms = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let clock_ms = ms.clone();
        let mut net = NetLoop::new(
            endpoint,
            Box::new(move || {
                Duration::from_millis(clock_ms.load(std::sync::atomic::Ordering::Relaxed))
            }),
            "test",
        );
        let pump = |net: &mut NetLoop| {
            let mut news = NetNews::default();
            for _ in 0..8 {
                ms.fetch_add(10, std::sync::atomic::Ordering::Relaxed);
                let (n, _) = net.poll_events(None, |_| {});
                news.started |= n.started;
                news.ended |= n.ended;
                net.end_poll(None);
            }
            news
        };
        pump(&mut net);

        net.endpoint.as_host_mut().unwrap().start(0);
        net.begin_map();
        net.map_loaded();
        let news = pump(&mut net);
        assert!(news.started && net.started);
        net.map_over();
        let news = pump(&mut net);
        assert!(net.end_answered && news.ended && net.ended);

        net.endpoint.as_host_mut().unwrap().start(1);
        net.begin_map();
        assert!(!net.started && !net.end_answered && !net.ended);
        net.endpoint
            .as_host_mut()
            .unwrap()
            .begin_barrier(BarrierKind::Start);
        net.over_at = Some(net.elapsed());
        let news = pump(&mut net);
        assert!(!net.end_answered && !net.pregame_answered && !news.started && !news.ended);

        net.map_loaded();
        let news = pump(&mut net);
        assert!(net.pregame_answered && news.started && net.started && !net.ended);

        net.map_over();
        let news = pump(&mut net);
        assert!(net.end_answered && news.ended && net.ended);
    }
}
