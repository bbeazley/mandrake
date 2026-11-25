use mandrake_sim::engine_rng::EngineWorld;
use mandrake_sim::statistics::Statistics;
use mandrake_tag::Rect;
use mandrake_tag::font::{self, Bitmap32, TextClip, text_flags};

use crate::app::ClientApp;
use crate::command_bar::{self, PromptTags};
use crate::{media, unit_info};
use mandrake_sim::net_game::DetachUnits;
use tracing::info;

pub(crate) const SELECTION_MODE_DETACH: i16 = 7;
pub(crate) const DETACH_PROMPTS: (usize, usize) = (14, 15);

const ROW_HEIGHT: i32 = 22;
const ROW_WIDTH: i32 = 290;
const ROW_RIGHT_INSET: i32 = 0x126;
const ROW_PITCH: i32 = 0x1a;
const FADE_MS: i32 = 0xfa;
const BAND_TRANSPARENCY: u16 = 0x8000;
pub(crate) const MENU_TEAMMATES: i32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConsoleMenu {
    mode: i32,
    fading: bool,
    deadline: i32,
    in_mode: bool,
}

impl Default for ConsoleMenu {
    fn default() -> Self {
        Self {
            mode: -1,
            fading: false,
            deadline: 0,
            in_mode: false,
        }
    }
}

impl ConsoleMenu {
    pub(crate) fn set(&mut self, mode: i32, now: i32) {
        if mode == -1 && !self.fading {
            self.fading = self.mode != -1;
        } else {
            self.fading = false;
            self.mode = mode;
        }
        self.deadline = now + FADE_MS;
    }

    pub(crate) fn follow_mode(&mut self, selection_mode: i16, now: i32) {
        let in_mode = selection_mode == SELECTION_MODE_DETACH;
        if self.in_mode && !in_mode && self.mode != -1 && !self.fading {
            self.set(-1, now);
        }
        self.in_mode = in_mode;
    }

    pub(crate) fn shown(&mut self, now: i32) -> Option<(i32, u16, u16)> {
        let left = self.deadline - now;
        if self.fading && left <= 0 {
            self.fading = false;
            self.mode = -1;
            self.deadline = now + FADE_MS;
            return None;
        }
        if self.mode == -1 {
            return None;
        }
        if self.fading && (1..FADE_MS).contains(&left) {
            let f = -((left << 16) / FADE_MS);
            return Some((self.mode, ((f + 0x1ffff) / 2) as u16, (f + 0xffff) as u16));
        }
        Some((self.mode, BAND_TRANSPARENCY, 0))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct MenuRow {
    pub place: i32,
    pub others: i32,
    pub fill: [u16; 3],
    pub name: Vec<u8>,
    pub share: Vec<u8>,
}

pub(crate) fn menu_rows(w: &EngineWorld, statistics: &Statistics) -> Vec<MenuRow> {
    let local = w.local_player;
    let team = w.local_team();
    let players = &w.players;
    let others = usize::try_from(team)
        .ok()
        .and_then(|t| players.teams.get(t))
        .map_or(0, |t| i32::from(t.players) - 1);
    let team_ranking = statistics.team_rankings.iter().find(|r| r.team == team);
    let mut rows = Vec::new();
    for (i, p) in players.players.iter().enumerate() {
        if i16::from(p.team) != team || i as i16 == local || p.flags & 4 != 0 {
            continue;
        }
        let part = statistics
            .player_rankings
            .iter()
            .find(|r| usize::try_from(r.player).ok() == Some(i))
            .map_or(0, |r| r.survivors_cost);
        let total = team_ranking.map_or(0, |t| t.losses_cost.wrapping_add(t.survivors_cost));
        let share = if total != 0 {
            (i64::from(part) * 100 / i64::from(total)) as i32
        } else {
            0
        };
        let name = players.names.get(i).map_or(&[][..], |n| {
            &n[..n.iter().position(|&b| b == 0).unwrap_or(n.len())]
        });
        let mut line = format!("{}. ", rows.len() + 1).into_bytes();
        line.extend_from_slice(name);
        let c = players.colors.get(i).map_or([0; 4], |c| c[1]);
        rows.push(MenuRow {
            place: rows.len() as i32,
            others,
            fill: [c[0], c[1], c[2]],
            name: line,
            share: format!("{share}%").into_bytes(),
        });
    }
    rows
}

pub(crate) fn row_rect(frame: Rect, row: &MenuRow) -> Rect {
    let x = frame.right - ROW_RIGHT_INSET;
    let y = frame.bottom + (row.place - row.others - 1) * ROW_PITCH;
    Rect::new(y, x, y + ROW_HEIGHT, x + ROW_WIDTH)
}

fn draw_row(tags: &PromptTags, row: &MenuRow, background: u32) -> Bitmap32 {
    let mut bmp = Bitmap32::new(ROW_WIDTH, ROW_HEIGHT, background);
    let mut rect = Rect::new(0, 0, ROW_HEIGHT, ROW_WIDTH);
    let white = [0xffff; 3];
    let mut text = |rect: Rect, s: &[u8], flags: u32| {
        font::draw_string(
            &mut bmp,
            rect,
            TextClip::None,
            &tags.family,
            font::STYLE_PLAIN,
            s,
            white,
            flags,
            0,
            &tags.locale,
        );
    };
    text(rect, &row.share, text_flags::MIDDLE | text_flags::RIGHT);
    rect.right -= font::string_width(&tags.family, font::STYLE_PLAIN, &row.share, &tags.locale);
    text(rect, &row.name, text_flags::MIDDLE);
    bmp
}

pub(crate) fn row_rgba(tags: &PromptTags, row: &MenuRow) -> (u32, u32, Vec<u8>) {
    let black = draw_row(tags, row, 0xff00_0000);
    let white = draw_row(tags, row, 0xffff_ffff);
    (
        black.width as u32,
        black.height as u32,
        command_bar::coverage_rgba(&black, &white),
    )
}

pub(crate) fn pick(teammates: &[i16], number: usize, shift: bool) -> Option<i16> {
    let n = if shift && number + 10 < teammates.len() {
        number + 10
    } else {
        number
    };
    teammates.get(n).copied()
}

impl ClientApp {
    fn menu_now(&self) -> i32 {
        self.machine_clock.elapsed().as_millis() as i32
    }

    fn selected_monsters(&self) -> Vec<usize> {
        unit_info::selected_monsters(&self.sim.engine, &self.selected_unit_ids)
    }

    pub(super) fn detach_press(&mut self) {
        self.clear_local_order_marks(false);
        let game = self.sim.engine.mode != 0;
        if self.selected_unit_ids.is_empty() || self.selection_machine.looking() || !game {
            return;
        }
        let monsters = self.selected_monsters();
        let what = self.sim.detach_units(&monsters);
        info!(
            "Detach Units: {} selected, {what:?} (teammates {:?})",
            monsters.len(),
            self.sim.teammates()
        );
        match what {
            DetachUnits::Nothing => {}
            DetachUnits::To(player) => self.detach_to(player),
            DetachUnits::Choose(_) => {
                let at = self.menu_now();
                if let Some(h) = self.hud.as_mut() {
                    let (left, right) = h
                        .prompt_tags()
                        .map(|t| t.selection_prompts(DETACH_PROMPTS))
                        .unwrap_or_default();
                    h.console.set(&left, &right, SELECTION_MODE_DETACH);
                }
                self.detach_menu.set(MENU_TEAMMATES, at);
                self.detach_menu.follow_mode(SELECTION_MODE_DETACH, at);
            }
        }
    }

    fn detach_to(&mut self, player: i16) {
        let look = self.selection_machine.looking();
        if self.selected_unit_ids.is_empty() || look {
            return;
        }
        let monsters = self.selected_monsters();
        let queued = self.sim.queue_detach(&monsters, player, look);
        let local = self.sim.engine.local_player;
        let mut owners = std::collections::BTreeMap::<i16, usize>::new();
        {
            let w = &self.sim.engine;
            for m in w.monsters.iter().filter(|m| m.identifier != -1) {
                *owners.entry(m.owner).or_default() += 1;
            }
        }
        info!(
            "Detach Units: DETACH of {} units to player {player} queued {queued} at tick {} \
             (local player {local}; monsters by owner now {owners:?})",
            monsters.len(),
            self.sim.current_tick
        );
        let now = media::local_time(self.sim.current_tick);
        let selection = self.selected_unit_ids.clone();
        command_bar::bar_tail(
            &mut self.sim,
            self.hud.as_mut(),
            &mut self.local_sprites,
            &mut self.interface.order_hide,
            &selection,
            now,
        );
        let at = self.menu_now();
        self.detach_menu.set(-1, at);
        if player != local {
            self.selection_machine.stop_looking();
            self.set_selection(Vec::new());
        }
    }

    pub(super) fn detach_number(&mut self, number: usize, shift: bool) -> bool {
        if self.selection_mode() != SELECTION_MODE_DETACH {
            return false;
        }
        let teammates = self.sim.teammates();
        let Some(player) = pick(&teammates, number, shift) else {
            return true;
        };
        info!("Detach Units: number {number} (Shift {shift}) picks player {player}");
        self.detach_to(player);
        self.clear_local_order_marks(true);
        let at = self.menu_now();
        self.detach_menu.set(-1, at);
        true
    }

    pub(super) fn detach_mouse_exit(&mut self) {
        self.clear_local_order_marks(true);
        let at = self.menu_now();
        self.detach_menu.set(-1, at);
        self.detach_menu
            .follow_mode(command_bar::SELECTION_MODE_NORMAL, at);
        info!("Detach Units: a mouse press ends mode 7");
    }
}

pub(crate) fn detach_this_tick(sim: &mandrake_sim::Simulation) -> bool {
    sim.commands_by_tick
        .get(&sim.current_tick)
        .into_iter()
        .flatten()
        .any(|c| c.command_type == mandrake_rec::CommandType::Detach)
}

pub(crate) fn log_owners(sim: &mandrake_sim::Simulation) {
    let w = &sim.engine;
    let mut owners = std::collections::BTreeMap::<i16, usize>::new();
    for m in w.monsters.iter().filter(|m| m.identifier != -1) {
        *owners.entry(m.owner).or_default() += 1;
    }
    info!(
        "Detach Units: a DETACH ran in tick {}: monsters by owner {owners:?}",
        sim.current_tick - 1
    );
}

#[derive(Default)]
pub(crate) struct MenuImages {
    key: Option<(Vec<MenuRow>, Rect)>,
    rows: Vec<(Rect, [u8; 3], mandrake_render::overlay::OverlayImage)>,
    white: Option<mandrake_render::overlay::OverlayImage>,
}

impl MenuImages {
    pub(crate) fn update(
        &mut self,
        renderer: &mandrake_render::overlay::OverlayRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        tags: &PromptTags,
        rows: Vec<MenuRow>,
        frame: Rect,
    ) {
        if self.white.is_none() {
            self.white = Some(renderer.create_image(device, queue, 1, 1, &[0xff; 4]));
        }
        let key = (rows, frame);
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.rows = key
            .0
            .iter()
            .map(|row| {
                let (w, h, rgba) = row_rgba(tags, row);
                let fill = row.fill.map(|c| (c >> 8) as u8);
                (
                    row_rect(frame, row),
                    fill,
                    renderer.create_image(device, queue, w, h, &rgba),
                )
            })
            .collect();
        self.key = Some(key);
    }

    pub(crate) fn draws(
        &self,
        band: u16,
        text: u16,
    ) -> Vec<mandrake_render::overlay::OverlayDraw<'_>> {
        use mandrake_render::overlay::{OverlayDraw, OverlayLayer, transparency_alpha};
        let layer = OverlayLayer::PostRasterization(1);
        let Some(white) = &self.white else {
            return Vec::new();
        };
        let mut draws = Vec::new();
        for (r, fill, image) in &self.rows {
            draws.push(OverlayDraw::fill(
                layer,
                white,
                [r.left, r.top, r.right - r.left, r.bottom - r.top],
                *fill,
                transparency_alpha(band),
            ));
            draws.push(OverlayDraw::at(
                layer,
                image,
                r.left,
                r.top,
                transparency_alpha(text),
            ));
        }
        draws
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_shows_until_a_change_then_fades_for_250_ms() {
        let mut m = ConsoleMenu::default();
        assert_eq!(m.shown(0), None);
        m.set(MENU_TEAMMATES, 100);
        assert_eq!(m.shown(5000), Some((0, 0x8000, 0)));
        m.set(-1, 5000);
        assert_eq!(m.shown(5000), Some((0, 0x8000, 0)));
        let (_, band, text) = m.shown(5125).unwrap();
        assert_eq!((band, text), (0xbfff, 0x7fff));
        let (_, band, text) = m.shown(5249).unwrap();
        assert!(band > 0xff00 && text > 0xfe00, "{band:x} {text:x}");
        assert_eq!(m.shown(5250), None);
        assert_eq!(m.shown(5251), None);
    }

    #[test]
    fn a_pick_ends_the_menu_at_once() {
        let mut m = ConsoleMenu::default();
        m.set(MENU_TEAMMATES, 0);
        m.set(-1, 1000);
        m.set(-1, 1000);
        assert_eq!(m.shown(1000), None);
        m.set(MENU_TEAMMATES, 2000);
        m.set(-1, 2000);
        m.set(MENU_TEAMMATES, 2100);
        assert_eq!(m.shown(2200), Some((0, 0x8000, 0)));
    }

    #[test]
    fn leaving_mode_7_elsewhere_fades_the_menu() {
        let mut m = ConsoleMenu::default();
        m.set(MENU_TEAMMATES, 0);
        m.follow_mode(SELECTION_MODE_DETACH, 10);
        m.follow_mode(SELECTION_MODE_DETACH, 20);
        assert_eq!(m.shown(20), Some((0, 0x8000, 0)));
        m.follow_mode(command_bar::SELECTION_MODE_NORMAL, 30);
        assert!(m.shown(200).is_some_and(|s| s.2 != 0));
        assert_eq!(m.shown(280), None);
    }

    #[test]
    fn a_number_picks_the_teammate_in_player_order_and_shift_adds_ten() {
        let mates = [0, 2, 3];
        assert_eq!(pick(&mates, 0, false), Some(0));
        assert_eq!(pick(&mates, 2, false), Some(3));
        assert_eq!(pick(&mates, 3, false), None);
        assert_eq!(pick(&mates, 1, true), Some(2));
        let many: Vec<i16> = (1..=13).collect();
        assert_eq!(pick(&many, 1, true), Some(12));
        assert_eq!(pick(&many, 2, true), Some(13));
        assert_eq!(pick(&many, 3, true), Some(4));
        assert_eq!(pick(&many, 9, false), Some(10));
    }

    #[test]
    fn rows_stand_above_the_view_bottom_right() {
        let frame = Rect::new(20, 0, 700, 1024);
        let row = |place| MenuRow {
            place,
            others: 2,
            fill: [0; 3],
            name: Vec::new(),
            share: Vec::new(),
        };
        assert_eq!(row_rect(frame, &row(0)), Rect::new(622, 730, 644, 1020));
        assert_eq!(row_rect(frame, &row(1)), Rect::new(648, 730, 670, 1020));
    }

    #[test]
    fn backslash_is_the_detach_units_key() {
        use crate::controls::{Binding, ControlAction, Controls};
        let backslash = Binding::parse("Backslash").unwrap();
        let controls = Controls::default();
        assert_eq!(controls.bindings(ControlAction::DetachUnits), [backslash]);
        assert_eq!(ControlAction::DetachUnits.config_name(), "detach_units");
    }
}
