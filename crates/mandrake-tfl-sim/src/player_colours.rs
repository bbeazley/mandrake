use mandrake_rec::tfl::TflFilmPlayer;

pub type GameColours = ([u16; 4], [u16; 4]);

const DISTINCT_COLOURS: [[u16; 4]; 20] = [
    [0xe146, 0, 0, 0],
    [0, 0xbd6f, 0, 0],
    [0x7fff, 0x2b84, 0, 0],
    [0xf0a2, 0xf850, 0, 0],
    [0xbae0, 0xbae0, 0xffff, 0],
    [0x547a, 0x51eb, 0x6666, 0],
    [0, 0xd47a, 0xd47a, 0],
    [0, 0, 0xee13, 0],
    [0xae13, 0xffff, 0xffff, 0],
    [0xd47a, 0x5709, 0xffff, 0],
    [0xdeb7, 0xb851, 0, 0],
    [0x6666, 0x6666, 0, 0],
    [0, 0xa665, 0xca3c, 0],
    [0xffff, 0xa3d6, 0xa3d6, 0],
    [0xca3c, 0x9709, 0xffff, 0],
    [0, 0x3851, 0x8a3c, 0],
    [0x6b84, 0, 0x6147, 0],
    [0xffff, 0xffff, 0xcf5b, 0],
    [0x35c2, 0x63d6, 0, 0],
    [0xbfff, 0xc28e, 0xa665, 0],
];

fn too_close(a: &[u16; 4], b: &[u16; 4]) -> bool {
    let d = |k: usize| (i32::from(a[k]) - i32::from(b[k])) >> 4;
    let (r, g, b) = (d(0), d(1), d(2));
    g * g + 2 * b * b + r * r < 0x1c3900
}

struct Entry {
    net_id: i32,
    captain: i32,
    primary: [u16; 4],
    secondary: [u16; 4],
}

fn make_distinct(players: &mut [Entry], list: &[usize]) {
    for i in (0..list.len()).rev() {
        let me = list[i];
        let mut next = (players[me].net_id as u32).wrapping_mul(0x2e);
        let mut tries = 0u32;
        loop {
            let close = (0..list.len())
                .rev()
                .any(|j| j != i && too_close(&players[me].primary, &players[list[j]].primary));
            if !close || tries >= 20 {
                break;
            }
            tries += 1;
            players[me].primary = DISTINCT_COLOURS[(next % 20) as usize];
            next = next.wrapping_add(1);
        }
    }
}

fn leads(p: &Entry) -> bool {
    p.captain == p.net_id || p.captain == -1
}

pub fn game_colours(players: &[TflFilmPlayer]) -> Vec<GameColours> {
    let mut e: Vec<Entry> = players
        .iter()
        .map(|p| Entry {
            net_id: i32::from(p.net_id),
            captain: i32::from(p.captain),
            primary: p.primary,
            secondary: p.secondary,
        })
        .collect();
    let leaders: Vec<usize> = (0..e.len()).filter(|&i| leads(&e[i])).collect();
    if !leaders.is_empty() {
        make_distinct(&mut e, &leaders);
    }
    let ids: Vec<i32> = e.iter().map(|p| p.net_id).collect();
    for id in ids {
        let mut list: Vec<usize> = Vec::new();
        if id != -1 {
            list.extend((0..e.len()).filter(|&i| e[i].net_id == id));
        }
        list.extend((0..e.len()).filter(|&i| {
            let p = &e[i];
            (id == -1 && leads(p)) || (p.captain == id && p.net_id != id)
        }));
        if !list.is_empty() {
            make_distinct(&mut e, &list);
        }
    }
    for i in 0..e.len() {
        let c = e[i].captain;
        if c == -1 {
            continue;
        }
        if let Some(k) = e.iter().position(|q| q.net_id == c) {
            e[i].secondary = e[i].primary;
            e[i].primary = e[k].primary;
        }
    }
    e.into_iter().map(|p| (p.primary, p.secondary)).collect()
}

pub fn set_game_colours(players: &mut [TflFilmPlayer]) {
    let colours = game_colours(players);
    for (p, (primary, secondary)) in players.iter_mut().zip(colours) {
        p.primary = primary;
        p.secondary = secondary;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(net_id: i16, captain: i16, primary: [u16; 4], secondary: [u16; 4]) -> TflFilmPlayer {
        TflFilmPlayer {
            slot: 0,
            name: String::new(),
            team_name: String::new(),
            primary,
            secondary,
            team: -1,
            field_02: 0,
            flags: 1,
            captain,
            net_id,
            field_0a: 0,
            metaserver_id: u32::MAX,
            field_10: [0; 8],
            appearance_00: 0,
            appearance_02: -1,
        }
    }

    const MAROON: [u16; 4] = [0x8000, 0, 0x2000, 0];
    const YELLOW: [u16; 4] = [0xffff, 0xffff, 0, 0];
    const BLUE: [u16; 4] = [0, 0x4000, 0xffff, 0];
    const ORANGE: [u16; 4] = [0xffff, 0x8000, 0, 0];

    #[test]
    fn a_follower_takes_its_captains_primary() {
        let players = [player(0, 0, MAROON, YELLOW), player(1, 0, BLUE, ORANGE)];
        assert_eq!(game_colours(&players), [(MAROON, MAROON), (MAROON, BLUE)]);
    }

    #[test]
    fn no_teams_keeps_the_players_colours() {
        let players = [player(0, -1, MAROON, YELLOW), player(1, -1, BLUE, ORANGE)];
        assert_eq!(game_colours(&players), [(MAROON, YELLOW), (BLUE, ORANGE)]);
    }

    #[test]
    fn a_leader_too_close_to_another_takes_the_tables_colour() {
        let players = [player(0, -1, BLUE, YELLOW), player(1, -1, BLUE, ORANGE)];
        assert_eq!(
            game_colours(&players),
            [(BLUE, YELLOW), (DISTINCT_COLOURS[6], ORANGE)]
        );
    }

    #[test]
    fn a_follower_like_its_captain_is_made_distinct() {
        let players = [player(0, 0, MAROON, YELLOW), player(1, 0, MAROON, ORANGE)];
        assert_eq!(
            game_colours(&players),
            [(MAROON, MAROON), (MAROON, DISTINCT_COLOURS[6])]
        );
    }
}
