use mandrake_net::game_kind::GameKind;

pub const NAME: &[u8] = b"Autohost";
const SETTLE_MS: u64 = 5000;
const COUNTDOWN_MS: u64 = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seat {
    pub id: i32,
    pub team: i32,
    pub ready: bool,
}

pub fn counts_ready(kind: GameKind, ready: u32) -> bool {
    kind == GameKind::Tfl || ready != 0
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Say(String),
    Start { unready: Vec<i32> },
}

#[derive(Debug, Clone, Default)]
pub struct Autohost {
    opened: u64,
    countdown: Option<(u64, u64)>,
}

impl Autohost {
    pub fn new(now: u64) -> Autohost {
        Autohost {
            opened: now,
            countdown: None,
        }
    }

    pub fn holds(seats: &[Seat], local: i32, cooperative: bool) -> bool {
        let counted: Vec<&Seat> = seats
            .iter()
            .filter(|s| s.id != local && s.team != local)
            .collect();
        let n = counted.len() as i32;
        let mut ready = 0;
        let mut sides = 0;
        for s in &counted {
            if s.ready {
                ready += 1;
                if s.team == -1 {
                    sides += 1;
                }
            }
            if s.id == s.team && seats.iter().any(|o| o.team == s.id && o.ready) {
                sides += 1;
            }
        }
        let need = ((n + 1) / 2).max(2);
        ready >= need && sides >= if cooperative { 1 } else { 2 }
    }

    pub fn abort(&mut self) -> Option<Action> {
        self.countdown
            .take()
            .map(|_| Action::Say("Game start aborted.".into()))
    }

    pub fn poll(&mut self, now: u64, seats: &[Seat], local: i32, cooperative: bool) -> Vec<Action> {
        let mut out = Vec::new();
        if Self::holds(seats, local, cooperative) && now.saturating_sub(self.opened) >= SETTLE_MS {
            match self.countdown {
                None => {
                    self.countdown = Some((now, 10));
                    out.push(Action::Say("Game will begin in 20 seconds...".into()));
                }
                Some((at, _)) if now - at >= COUNTDOWN_MS => {
                    self.countdown = None;
                    out.push(Action::Say("Starting game...".into()));
                    out.push(Action::Start {
                        unready: seats
                            .iter()
                            .filter(|s| !s.ready && s.id != local)
                            .map(|s| s.id)
                            .collect(),
                    });
                }
                Some((at, next)) if now - at >= next * 1000 => {
                    self.countdown = Some((at, next + 5));
                    out.push(Action::Say(format!(
                        "Game will begin in {} seconds...",
                        20 - next
                    )));
                }
                Some(_) => {}
            }
        } else if self.countdown.take().is_some() {
            out.push(Action::Say("Game start aborted.".into()));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seat(id: i32, team: i32, ready: bool) -> Seat {
        Seat { id, team, ready }
    }

    #[test]
    fn half_ready_with_at_least_two_and_two_sides() {
        let host = seat(0, -1, true);
        assert!(
            !Autohost::holds(&[host, seat(1, -1, true)], 0, false),
            "one ready"
        );
        assert!(Autohost::holds(
            &[host, seat(1, -1, true), seat(2, -1, true)],
            0,
            false
        ));
        let three = [
            host,
            seat(1, -1, true),
            seat(2, -1, false),
            seat(3, -1, false),
        ];
        assert!(!Autohost::holds(&three, 0, false));
        let three = [
            host,
            seat(1, -1, true),
            seat(2, -1, true),
            seat(3, -1, false),
        ];
        assert!(Autohost::holds(&three, 0, false));
        let team = [host, seat(1, 1, true), seat(2, 1, true)];
        assert!(!Autohost::holds(&team, 0, false));
        assert!(Autohost::holds(&team, 0, true));
    }

    #[test]
    fn the_countdown_says_its_lines_then_starts_without_the_unready() {
        let seats = [
            seat(0, -1, true),
            seat(1, -1, true),
            seat(2, -1, true),
            seat(3, -1, false),
        ];
        let mut a = Autohost::new(0);
        assert!(
            a.poll(4000, &seats, 0, false).is_empty(),
            "the lobby settles 5 s"
        );
        let say = |s: &str| vec![Action::Say(s.into())];
        assert_eq!(
            a.poll(5000, &seats, 0, false),
            say("Game will begin in 20 seconds...")
        );
        assert!(a.poll(14_999, &seats, 0, false).is_empty());
        assert_eq!(
            a.poll(15_000, &seats, 0, false),
            say("Game will begin in 10 seconds...")
        );
        assert_eq!(
            a.poll(20_000, &seats, 0, false),
            say("Game will begin in 5 seconds...")
        );
        assert_eq!(
            a.poll(25_000, &seats, 0, false),
            vec![
                Action::Say("Starting game...".into()),
                Action::Start { unready: vec![3] }
            ]
        );
    }

    fn tfl(id: i32, team: i32) -> Seat {
        Seat {
            id,
            team,
            ready: counts_ready(GameKind::Tfl, 0),
        }
    }

    #[test]
    fn tfl_players_start_after_the_countdown_with_no_ready() {
        assert!(!counts_ready(GameKind::Myth2, 0));
        assert!(counts_ready(GameKind::Myth2, 1));
        let server = tfl(0, -1);
        assert!(!Autohost::holds(&[server, tfl(1, -1)], 0, false));
        let seats = [server, tfl(1, -1), tfl(2, -1)];
        let mut a = Autohost::new(0);
        assert!(a.poll(4999, &seats, 0, false).is_empty());
        let say = |s: &str| vec![Action::Say(s.into())];
        assert_eq!(
            a.poll(5000, &seats, 0, false),
            say("Game will begin in 20 seconds...")
        );
        assert_eq!(
            a.poll(15_000, &seats, 0, false),
            say("Game will begin in 10 seconds...")
        );
        assert_eq!(
            a.poll(20_000, &seats, 0, false),
            say("Game will begin in 5 seconds...")
        );
        assert_eq!(
            a.poll(25_000, &seats, 0, false),
            vec![
                Action::Say("Starting game...".into()),
                Action::Start { unready: vec![] }
            ],
            "no one removed"
        );
        let mut a = Autohost::new(0);
        assert_eq!(a.poll(5000, &seats, 0, false).len(), 1);
        assert_eq!(
            a.poll(6000, &seats[..2], 0, false),
            say("Game start aborted.")
        );
    }

    #[test]
    fn tfl_coop_starts_with_one_side() {
        let team = [tfl(0, -1), tfl(1, 1), tfl(2, 1)];
        assert!(Autohost::holds(&team, 0, true));
        assert!(!Autohost::holds(&team, 0, false));
        let coop = [tfl(0, -1), tfl(1, -1), tfl(2, -1)];
        assert!(Autohost::holds(&coop, 0, true));
    }

    #[test]
    fn a_player_unreadying_aborts_the_countdown() {
        let mut seats = [seat(0, -1, true), seat(1, -1, true), seat(2, -1, true)];
        let mut a = Autohost::new(0);
        assert_eq!(a.poll(6000, &seats, 0, false).len(), 1);
        seats[2].ready = false;
        assert_eq!(
            a.poll(7000, &seats, 0, false),
            vec![Action::Say("Game start aborted.".into())]
        );
        assert!(a.poll(8000, &seats, 0, false).is_empty());
    }

    #[test]
    fn abort_clears_active_countdown() {
        let seats = [seat(0, -1, true), seat(1, -1, true), seat(2, -1, true)];
        let mut a = Autohost::new(0);
        assert_eq!(a.abort(), None);
        assert_eq!(a.poll(6000, &seats, 0, false).len(), 1);
        assert_eq!(a.abort(), Some(Action::Say("Game start aborted.".into())));
        assert_eq!(a.abort(), None);
    }
}
