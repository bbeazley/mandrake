pub use mandrake_common::game_options::ALLIANCES as GAME_OPTION_ALLIANCES;
pub const GAME_SOUND_ALLIED: u16 = 0x1a;
pub const GAME_SOUND_ALLIANCE_BROKEN: u16 = 0x1b;
pub const MAX_TEAMS: usize = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Alliances {
    rows: [u16; MAX_TEAMS],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AllianceChange {
    pub team: i16,
    pub other: i16,
    pub allied: bool,
}

impl Alliances {
    fn get(&self, a: i16, b: i16) -> bool {
        match (usize::try_from(a), usize::try_from(b)) {
            (Ok(a), Ok(b)) if a < MAX_TEAMS && b < MAX_TEAMS => self.rows[a] & (1 << b) != 0,
            _ => false,
        }
    }

    pub fn teams_are_allied(&self, a: i16, b: i16, teams: usize) -> bool {
        if a == b {
            return true;
        }
        if a < 0 || b < 0 || usize::from(a.max(b) as u16) >= teams {
            return false;
        }
        self.get(a, b)
    }

    pub fn ally_with_team(
        &mut self,
        a: i16,
        b: i16,
        allied: bool,
        teams: usize,
        options: u32,
    ) -> Option<AllianceChange> {
        if options & GAME_OPTION_ALLIANCES == 0 || a < 0 || b < 0 {
            return None;
        }
        if usize::from(a.max(b) as u16) >= teams || usize::from(a.max(b) as u16) >= MAX_TEAMS {
            return None;
        }
        let current = a == b || self.get(a, b);
        if current == allied {
            return None;
        }
        let (row, bit) = (a as usize, 1u16 << b);
        if allied {
            self.rows[row] |= bit;
        } else {
            self.rows[row] &= !bit;
        }
        Some(AllianceChange {
            team: a,
            other: b,
            allied,
        })
    }

    pub fn break_all(&mut self, teams: usize) -> bool {
        let n = teams.min(MAX_TEAMS);
        let any = (0..n).any(|i| (0..n).any(|j| i != j && self.rows[i] & (1 << j) != 0));
        if any {
            self.rows = [0; MAX_TEAMS];
        }
        any
    }

    pub fn any(&self) -> bool {
        self.rows.iter().any(|&r| r != 0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AllianceView {
    pub alliances: Alliances,
    pub teams: usize,
}

impl AllianceView {
    pub fn teams_are_allied(&self, a: i16, b: i16) -> bool {
        self.alliances.teams_are_allied(a, b, self.teams)
    }
}

pub fn ally_command_teams(data: &[u8]) -> Option<(i16, i16)> {
    let word = |at: usize| {
        data.get(at..at + 2)
            .map(|b| i16::from_be_bytes([b[0], b[1]]))
    };
    Some((word(4)?, word(6)?))
}

pub fn ally_command_data(team: i16, other: i16) -> Vec<u8> {
    let mut d = vec![0u8; 4];
    d.extend_from_slice(&team.to_be_bytes());
    d.extend_from_slice(&other.to_be_bytes());
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_matrix_is_one_sided_and_needs_option_0x2000() {
        let mut a = Alliances::default();
        assert!(a.teams_are_allied(2, 2, 3));
        assert!(a.teams_are_allied(-1, -1, 3));
        assert!(!a.teams_are_allied(0, -1, 3));
        assert_eq!(a.ally_with_team(0, 1, true, 3, 0), None, "no option 0x2000");
        let c = a.ally_with_team(0, 1, true, 3, GAME_OPTION_ALLIANCES);
        assert_eq!(
            c,
            Some(AllianceChange {
                team: 0,
                other: 1,
                allied: true
            })
        );
        assert!(a.teams_are_allied(0, 1, 3));
        assert!(!a.teams_are_allied(1, 0, 3), "one-sided");
        assert!(!a.teams_are_allied(0, 1, 1), "beyond the team count");
        assert_eq!(a.ally_with_team(0, 1, true, 3, GAME_OPTION_ALLIANCES), None);
        assert_eq!(a.ally_with_team(0, 3, true, 3, GAME_OPTION_ALLIANCES), None);
        assert_eq!(
            a.ally_with_team(1, 1, false, 3, GAME_OPTION_ALLIANCES)
                .map(|c| c.allied),
            Some(false)
        );
        assert!(a.teams_are_allied(1, 1, 3), "a team stays its own ally");
        assert!(a.break_all(3));
        assert!(!a.teams_are_allied(0, 1, 3));
        assert!(!a.break_all(3));
    }

    #[test]
    fn the_command_data_round_trips() {
        let d = ally_command_data(2, 0);
        assert_eq!(d, [0, 0, 0, 0, 0, 2, 0, 0]);
        assert_eq!(ally_command_teams(&d), Some((2, 0)));
        assert_eq!(ally_command_teams(&d[..6]), None);
    }
}
