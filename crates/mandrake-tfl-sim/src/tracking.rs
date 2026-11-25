use crate::TflSim;
use crate::draw_site;
use crate::groups::distance;
use crate::trig::arctangent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterestEvent {
    pub site: u32,
    pub tick: u32,
    pub object: usize,
    pub identifier: i16,
    pub interest: u8,
    pub point: [i32; 3],
    pub velocity: [i16; 3],
    pub visible: bool,
    pub player: i16,
}

pub mod site {
    pub const SET_MONSTER_ACTION: u32 = crate::draw_site::TRACKING_SET_MONSTER_ACTION;
    pub const ATTACKING: u32 = crate::draw_site::TRACKING_ATTACKING;
    pub const DODGE: u32 = crate::draw_site::TRACKING_DODGE;
    pub const KILL_MONSTER: u32 = crate::draw_site::TRACKING_KILL_MONSTER;
    pub const ATTACK: u32 = crate::draw_site::TRACKING_ATTACK;
    pub const NEW_PATH: u32 = crate::draw_site::TRACKING_NEW_PATH;
    pub const DAMAGE_1: u32 = crate::draw_site::TRACKING_DAMAGE_1;
    pub const DAMAGE_2: u32 = crate::draw_site::TRACKING_DAMAGE_2;
    pub const DAMAGE_5: u32 = crate::draw_site::TRACKING_DAMAGE_5;
    pub const DAMAGE_STUN: u32 = crate::draw_site::TRACKING_DAMAGE_STUN;
    pub const DAMAGE_6: u32 = crate::draw_site::TRACKING_DAMAGE_6;
    pub const DAMAGE_KIND_6: u32 = crate::draw_site::TRACKING_DAMAGE_KIND_6;
    pub const NEW_OBJECT: u32 = crate::draw_site::TRACKING_NEW_OBJECT;
}

impl TflSim {
    pub fn take_interest_events(&mut self) -> Vec<InterestEvent> {
        std::mem::take(&mut self.interest_events)
    }

    pub fn object_visible(&self, object: usize) -> bool {
        let o = &self.objects[object];
        o.flags & 0x200 == 0 && self.sight.visible_bits(o.x >> 10, o.y >> 10) & 1 != 0
    }

    pub(crate) fn set_interest(&mut self, site: u32, object: usize, interest: u8, player: i16) {
        self.objects[object].interest = interest;
        self.interest_event(site, object, player);
    }

    pub(crate) fn interest_event(&mut self, site: u32, object: usize, player: i16) {
        if self.camera_control_flags & 4 == 0 {
            return;
        }
        let o = &self.objects[object];
        let event = InterestEvent {
            site,
            tick: self.tick,
            object,
            identifier: o.identifier,
            interest: o.interest,
            point: [o.x, o.y, o.z],
            velocity: o.velocity,
            visible: self.object_visible(object),
            player,
        };
        self.interest_events.push(event);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObserverRequest {
    Center([i32; 3]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackingPoll {
    Hold,
    Move { dx: i16, dy: i16 },
    Turn { left: bool },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tracking {
    pub object: i16,
    pub identifier: i16,
    pub point: [i32; 3],
    pub interest: i16,
    pub team: i16,
    pub countdown: i32,
    pub hold: i32,
    pub side: i32,
    pub turning: i32,
    pub turn_ticks: i32,
    pub velocity: [i16; 3],
}

fn countdown_draw(site: u32, random: &mut dyn FnMut(u32) -> u16) -> i32 {
    ((i32::from(random(site)) * 0x78) >> 16) + 0x78
}

impl Tracking {
    pub fn new_map(
        &mut self,
        flags: u16,
        sim: &TflSim,
        random: &mut dyn FnMut(u32) -> u16,
    ) -> Option<ObserverRequest> {
        self.interest = 0;
        self.team = -1;
        self.turn_ticks = 0;
        self.countdown = countdown_draw(draw_site::TRACKING_NEW_MAP_COUNTDOWN, random);
        self.turning = 0;
        self.hold = 0;
        if flags & 4 != 0 {
            self.retarget(sim, random)
        } else {
            None
        }
    }

    pub fn event(
        &mut self,
        e: &InterestEvent,
        random: &mut dyn FnMut(u32) -> u16,
    ) -> Option<ObserverRequest> {
        if self.hold != 0 || !e.visible || i16::from(e.interest) <= self.interest {
            return None;
        }
        self.object = e.object as i16;
        self.interest = i16::from(e.interest);
        self.identifier = e.identifier;
        self.team = e.player;
        self.turning = 0;
        self.velocity = e.velocity;
        self.point = e.point;
        Some(self.centre(draw_site::TRACKING_EVENT_CENTRE, random))
    }

    fn centre(&mut self, site: u32, random: &mut dyn FnMut(u32) -> u16) -> ObserverRequest {
        let r = ObserverRequest::Center(self.point);
        self.turn_ticks = 0;
        self.countdown = countdown_draw(site, random);
        self.hold = 0x3c;
        r
    }

    pub fn retarget(
        &mut self,
        sim: &TflSim,
        random: &mut dyn FnMut(u32) -> u16,
    ) -> Option<ObserverRequest> {
        if self.hold != 0 {
            return None;
        }
        self.interest = i16::MIN;
        self.choose(sim, random);
        Some(self.centre(draw_site::TRACKING_RETARGET_CENTRE, random))
    }

    pub fn choose(&mut self, sim: &TflSim, random: &mut dyn FnMut(u32) -> u16) {
        let count = sim.monsters.len() as i32;
        let start = if count < 1 {
            0
        } else {
            (i32::from(random(draw_site::TRACKING_CHOOSE)) * count) >> 16
        };
        let mut best = self.interest;
        let mut chosen = -1i32;
        let mut found = false;
        for i in (start..count).chain(0..start) {
            let m = &sim.monsters[i as usize];
            if m.identifier == 0 || !sim.object_visible(m.object) {
                continue;
            }
            found = false;
            if i16::from(m.team as i8) != self.team {
                let interest = i16::from(sim.objects[m.object].interest);
                if best <= interest {
                    best = interest;
                    chosen = i;
                    found = true;
                }
            }
        }
        if !found {
            return;
        }
        let m = &sim.monsters[chosen as usize];
        let o = &sim.objects[m.object];
        self.object = m.object as i16;
        self.identifier = o.identifier;
        self.team = i16::from(m.team as i8);
        if 0xc00 < distance((self.point[0], self.point[1]), (o.x, o.y)) as i32 {
            self.point = [o.x, o.y, o.z];
        }
        self.interest = best;
    }

    pub fn update(
        &mut self,
        ticks: i32,
        sim: &TflSim,
        random: &mut dyn FnMut(u32) -> u16,
    ) -> Vec<ObserverRequest> {
        let mut out = Vec::new();
        for _ in 0..ticks {
            if self.turn_ticks != 0 {
                self.turn_ticks -= 1;
                if self.turn_ticks == 0 {
                    self.countdown = countdown_draw(draw_site::TRACKING_COUNTDOWN, random);
                }
            }
            if self.countdown == 0 {
                continue;
            }
            self.countdown -= 1;
            if self.hold != 0 {
                self.hold -= 1;
            }
            if self.countdown != 0 {
                continue;
            }
            let r = u32::from(random(draw_site::TRACKING_UPDATE));
            if (r * 3) & 0xffff_0000 == 0 && self.turning == 0 {
                self.turning = 1;
                self.side = i32::from(random(draw_site::TRACKING_SIDE) & 1);
                self.turn_ticks =
                    ((i32::from(random(draw_site::TRACKING_TURN_TICKS)) * 0x3c) >> 16) + 0x1e;
            } else {
                self.turning = 0;
                out.extend(self.retarget(sim, random));
            }
        }
        out
    }

    pub fn poll(&mut self, sim: &TflSim, speed: i32) -> TrackingPoll {
        if self.object == -1 && self.identifier == -1 {
            if 0 < self.turn_ticks {
                return TrackingPoll::Turn {
                    left: self.side != 0,
                };
            }
            return TrackingPoll::Hold;
        }
        let o = usize::try_from(self.object)
            .ok()
            .and_then(|k| sim.objects.get(k))
            .filter(|o| o.identifier != 0 && o.identifier == self.identifier);
        let Some(o) = o else {
            self.object = -1;
            self.identifier = -1;
            return TrackingPoll::Hold;
        };
        let by = |v: i16| ((i32::from(v) * speed) >> 8) as i16;
        self.velocity = o.velocity.map(by);
        self.point = [o.x, o.y, o.z];
        TrackingPoll::Move {
            dx: self.velocity[0],
            dy: self.velocity[1],
        }
    }
}

pub fn centre_observer(
    target: [i32; 3],
    observer: (i32, i32),
    cells: (i32, i32),
) -> ((i32, i32), u16) {
    let (dx, dy) = (observer.0 - target[0], observer.1 - target[1]);
    let d =
        crate::groups::isqrt((dy.wrapping_mul(dy)).wrapping_add(dx.wrapping_mul(dx)) as u32) as i32;
    let (mut x, mut y) = if d != 0 {
        (
            dx.wrapping_mul(0x1400) / d + target[0],
            dy.wrapping_mul(0x1400) / d + target[1],
        )
    } else {
        (observer.0, observer.1)
    };
    x = x.clamp(0, (cells.0 * 0x400 - 1).max(0));
    y = y.clamp(0, (cells.1 * 0x400 - 1).max(0));
    ((x, y), arctangent(target[0] - x, target[1] - y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centre_observer_is_tfl_15s() {
        let cases = [
            ((55301, 36544), [24310, 77309, 1208], (27408, 73234), 23168),
            ((27408, 68864), [18107, 54943, 0], (20951, 59200), 43008),
            ((20261, 60166), [17433, 56004, 0], (20310, 60238), 42944),
            ((20022, 60688), [16586, 73653, 537], (17897, 68705), 19072),
            ((18573, 66524), [19493, 63892, 0], (17804, 68725), 52672),
            ((17804, 68725), [17105, 63329, 0], (17762, 68406), 47808),
            ((17762, 68406), [19372, 63884, 258], (17655, 68707), 52736),
            ((15531, 57816), [16569, 65000, 0], (15837, 59933), 14912),
            ((15837, 59933), [50327, 33160, 0], (46283, 36299), 58624),
            ((46790, 35475), [18433, 64097, 0], (22036, 60460), 24512),
            ((22036, 60460), [18433, 64097, 0], (22036, 60460), 24512),
        ];
        for (from, target, to, facing) in cases {
            assert_eq!(
                centre_observer(target, from, (0x100, 0x100)),
                (to, facing),
                "from {from:?} to {target:?}"
            );
        }
        assert_eq!(
            centre_observer([100, 100, 0], (-5000, 100), (4, 4)).0,
            (0, 100)
        );
    }
}
