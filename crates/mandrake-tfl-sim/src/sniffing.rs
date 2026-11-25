use crate::draw_site;
use crate::map_actions::index_of;
use crate::{Stop, TflSim};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputAction {
    pub flags: [u32; 4],
    pub key: i16,
}

impl Default for InputAction {
    fn default() -> Self {
        Self {
            flags: [0; 4],
            key: -1,
        }
    }
}

impl InputAction {
    pub fn with_actions(actions: &[u16]) -> Self {
        let mut a = Self::default();
        for &f in actions {
            if let Some(w) = a.flags.get_mut(usize::from(f >> 5)) {
                *w |= 1 << (f & 31);
            }
        }
        a
    }

    pub fn flag(&self, f: u32) -> bool {
        self.flags
            .get((f >> 5) as usize)
            .is_some_and(|w| w & (1 << (f & 31)) != 0)
    }

    pub fn has_action_flags(&self) -> bool {
        const WATCHED: [u32; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0x1e, 0x1f];
        WATCHED.iter().any(|&f| self.flag(f)) || (self.key != -1 && !(0..=15).contains(&self.key))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sniffed {
    Passed,
    Matched,
    Cleared,
}

impl Sniffed {
    pub fn survives(self) -> bool {
        self != Self::Cleared
    }
}

impl TflSim {
    pub fn handle_action_sniffing(&mut self, action: &InputAction) -> Result<Sniffed, Stop> {
        let case = self.sniffer.map_or(0, |s| s.case);
        if !(0x32..=0x37).contains(&case) || !action.has_action_flags() {
            return Ok(Sniffed::Passed);
        }
        let own: &[u32] = match case {
            0x32 => &[0x1e],
            0x33 => &[0, 1],
            0x34 => &[2, 3],
            0x35 => &[4, 5],
            0x36 => &[6, 7],
            _ => &[8, 9],
        };
        if own.iter().any(|&f| action.flag(f)) {
            self.sniffer_matched()?;
            return Ok(Sniffed::Matched);
        }
        if let Some(s) = self.sniffer
            && s.flags & 8 != 0
            && self.tick as i32 > s.deadline
        {
            self.sniffer_time_out(draw_site::SNIFFING_ACTION_TIME_OUT)?;
        }
        Ok(Sniffed::Cleared)
    }

    pub fn handle_command_sniffing(
        &mut self,
        command_type: u8,
        payload: &[u8],
    ) -> Result<bool, Stop> {
        let case = self.sniffer.map_or(0, |s| s.case);
        if !(0x64..=0xa2).contains(&case) || command_type <= 1 {
            return Ok(true);
        }
        let word = |o: usize| -> Option<u16> {
            payload
                .get(o..o + 2)
                .map(|b| u16::from_be_bytes([b[0], b[1]]))
        };
        let general = |code: u16| command_type == 2 && word(2) == Some(code);
        let target_flag = command_type == 4 && word(0).is_some_and(|f| f & 2 != 0);
        let movement = |formation: Option<u16>| {
            command_type == 3
                && word(0).is_some_and(|f| f & 8 != 0)
                && formation.is_none_or(|g| word(2) == Some(g))
        };
        let matched = match case {
            100..=104 => general((case - 100) as u16),
            105..=108 => i32::from(command_type) == i32::from(case) - 102,
            109 => command_type == 3 || command_type == 4 || general(0),
            150 | 151 if target_flag => {
                let list = |o: usize| -> Option<(usize, &[u8])> {
                    let n = word(o)? as i16;
                    let n = usize::try_from(n).unwrap_or(0);
                    Some((n, payload.get(o + 2..o + 2 + 2 * n)?))
                };
                match list(2).and_then(|(n, a)| Some((a, list(4 + 2 * n)?.1))) {
                    Some((units, targets)) => {
                        let n = units.len().min(targets.len());
                        (units[..n] == targets[..n]) == (case == 150)
                    }
                    None => false,
                }
            }
            152 => target_flag,
            160 => movement(Some(2)),
            161 => movement(Some(8)),
            162 => movement(None),
            _ => false,
        };
        if matched {
            self.sniffer_matched()?;
            return Ok(true);
        }
        if let Some(s) = self.sniffer
            && s.flags & 8 != 0
            && (s.deadline == -1 || self.tick as i32 > s.deadline)
        {
            self.sniffer_time_out(draw_site::SNIFFING_COMMAND_TIME_OUT)?;
        }
        Ok(false)
    }

    fn sniffer_matched(&mut self) -> Result<(), Stop> {
        let Some(s) = self.sniffer else {
            return Ok(());
        };
        if s.flags & 4 == 0
            && let Some(j) = index_of(&self.map_actions.actions, s.action)
        {
            self.succeed(j)?;
        }
        if let Some(s) = self.sniffer.as_mut() {
            if s.flags & 1 != 0 {
                s.case = 0;
            }
            if s.flags & 2 == 0 {
                s.flags &= !8;
            }
            s.flags |= 4;
        }
        Ok(())
    }

    fn sniffer_time_out(&mut self, site: u32) -> Result<(), Stop> {
        let Some(s) = self.sniffer else {
            return Ok(());
        };
        if let Some(j) = index_of(&self.map_actions.actions, s.action) {
            self.set_error(j)?;
        }
        let Some(s) = self.sniffer else {
            return Ok(());
        };
        let r = if s.range > 0 {
            (i32::from(self.rng.draw(site)) * s.range) >> 16
        } else {
            0
        };
        let deadline = r.wrapping_add(s.delay).wrapping_add(self.tick as i32);
        if let Some(s) = self.sniffer.as_mut() {
            s.deadline = deadline;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_watched_flags_and_keys() {
        assert!(!InputAction::default().has_action_flags());
        assert!(InputAction::with_actions(&[2]).has_action_flags());
        assert!(InputAction::with_actions(&[0x1f]).has_action_flags());
        assert!(!InputAction::with_actions(&[34, 37, 10]).has_action_flags());
        let key = |k| InputAction {
            key: k,
            ..InputAction::default()
        };
        assert!(!key(15).has_action_flags());
        assert!(key(16).has_action_flags());
    }
}
