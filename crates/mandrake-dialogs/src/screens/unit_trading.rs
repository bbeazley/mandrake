use mandrake_common::FourCC;
use mandrake_session::screen::UnitTrading;
use mandrake_session::{Action, Screen};
use mandrake_tag::font::{STYLE_BOLD, STYLE_PLAIN, text_flags};

use super::{OpenScreen, ScreenFrame};
use crate::core::{Dialog, DialogStack, format_one};
use crate::resources::Resources;

pub const UNTR: FourCC = FourCC(*b"untr");
const INUT: FourCC = FourCC(*b"inut");
const FLAGS: u16 = 8;

pub const OK: i16 = 1;
pub const CANCEL: i16 = 2;
pub const RESET: i16 = 3;
const HEADS: std::ops::RangeInclusive<i16> = 4..=11;
const LEFT_HEADS: u32 = 0x990;
const FIRST_ROW: i16 = 12;
pub const ROWS: usize = 8;
pub const POINTS: i16 = 44;

pub const fn count_item(k: usize) -> i16 {
    FIRST_ROW + 4 * k as i16 + 3
}

fn row_count_value(dialog: &Dialog, k: usize) -> i16 {
    dialog.arrow_value(count_item(k)) as i16
}

fn counts(dialog: &Dialog, rows: usize) -> Vec<i16> {
    (0..rows).map(|k| row_count_value(dialog, k)).collect()
}

fn counts_differ(dialog: &Dialog, rows: usize, sent: &[i16]) -> bool {
    sent.len() != rows || (0..rows).any(|k| row_count_value(dialog, k) != sent[k])
}

fn set_text(dialog: &mut Dialog, id: i16, text: &[u8], style: i16, flags: u32) {
    if let Some((item, t)) = dialog.control_mut(id).and_then(|c| c.static_text_mut()) {
        t.set_text(item, text);
        t.set_appearance(item, None, style, None, Some(flags));
    }
}

fn set_arrows(dialog: &mut Dialog, k: usize, min: i32, max: i32, value: i32) {
    if let Some((item, a)) = dialog
        .control_mut(count_item(k))
        .and_then(|c| c.double_arrow_mut())
    {
        a.set_value(item, min, max, value);
    }
}

pub struct UnitTradingScreen {
    points: i32,
    sent: Vec<i16>,
    line: Vec<u8>,
}

pub fn open(
    data: &UnitTrading,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let def = res.dialog(UNTR)?;
    let mut dialog = Dialog::new(&def, UNTR, FLAGS, res);
    dialog.set_background(res.art(INUT), 0);
    set_text(&mut dialog, POINTS, b"", STYLE_PLAIN, 0x24);
    for id in HEADS {
        let flags = if LEFT_HEADS >> id & 1 != 0 {
            text_flags::SHADOW
        } else {
            text_flags::SHADOW | text_flags::RIGHT
        };
        if let Some((item, t)) = dialog.control_mut(id).and_then(|c| c.static_text_mut()) {
            t.set_appearance(item, None, STYLE_BOLD, None, Some(flags));
        }
    }
    let mut points = 0;
    for k in 0..ROWS {
        let base = FIRST_ROW + 4 * k as i16;
        match data.rows.get(k).filter(|r| r.adjustable) {
            Some(row) => {
                set_text(
                    &mut dialog,
                    base,
                    &row.name.0,
                    STYLE_PLAIN,
                    text_flags::SHADOW,
                );
                let range = format!("{}/{}", row.minimum, row.maximum);
                set_text(
                    &mut dialog,
                    base + 2,
                    range.as_bytes(),
                    STYLE_PLAIN,
                    text_flags::RIGHT,
                );
                let cost = row.cost.to_string();
                set_text(
                    &mut dialog,
                    base + 1,
                    cost.as_bytes(),
                    STYLE_PLAIN,
                    text_flags::CENTER,
                );
                points += i32::from(row.placed) * i32::from(row.cost);
            }
            None => {
                for id in base..base + 4 {
                    dialog.set_visible(id, false);
                }
            }
        }
    }
    for (k, row) in data.rows.iter().enumerate().take(ROWS) {
        set_arrows(
            &mut dialog,
            k,
            i32::from(row.minimum),
            i32::from(row.maximum),
            i32::from(row.placed),
        );
    }
    let line = res.string_line(INUT, 0).to_vec();
    stack.push(dialog);
    let screen = UnitTradingScreen {
        points,
        sent: data.counts.clone(),
        line,
    };
    if let Some(dialog) = stack.find_mut(UNTR) {
        place(dialog, data, (0, 0));
        screen.update(dialog, data);
    }
    Some(Box::new(screen))
}

fn place(dialog: &mut Dialog, data: &UnitTrading, screen: (i32, i32)) {
    let b = dialog.bounds();
    dialog.move_to((screen.0 - b.width()) / 2, data.top);
}

fn format_points(line: &[u8], left: i32) -> Vec<u8> {
    format_one(line, left.to_string().as_bytes())
}

impl UnitTradingScreen {
    fn update(&self, dialog: &mut Dialog, data: &UnitTrading) {
        let n = data.rows.len().min(ROWS);
        let now = counts(dialog, n);
        let mut left = self.points;
        if n > 0 {
            let spent: i32 = data
                .rows
                .iter()
                .zip(&now)
                .filter(|(r, _)| r.adjustable)
                .map(|(r, &c)| i32::from(c) * i32::from(r.cost))
                .sum();
            left = i32::from((self.points - spent) as i16);
            for (k, (row, &c)) in data.rows.iter().zip(&now).enumerate().take(n) {
                let max = if row.cost == 0 {
                    i32::from(row.maximum)
                } else {
                    (left / i32::from(row.cost) + i32::from(c)).min(i32::from(row.maximum))
                };
                set_arrows(dialog, k, -1, max, -1);
            }
        }
        let text = format_points(&self.line, left);
        if let Some((item, t)) = dialog.control_mut(POINTS).and_then(|c| c.static_text_mut()) {
            t.set_text(item, &text);
        }
    }
}

impl OpenScreen for UnitTradingScreen {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Playing {
            overlay: Some(mandrake_session::Overlay::UnitTrading(data)),
        } = screen
        else {
            return;
        };
        let Some(dialog) = frame.stack.find_mut(UNTR) else {
            return;
        };
        let (w, h) = frame.input.size;
        place(dialog, data, (w as i32, h as i32));
        for hit in frame.hits.iter().filter(|h| h.dialog == UNTR) {
            match hit.item {
                OK => frame.out.actions.push(Action::Ok),
                CANCEL => frame.out.actions.push(Action::Cancel),
                RESET => {
                    for (k, row) in data.rows.iter().enumerate().take(ROWS) {
                        set_arrows(
                            dialog,
                            k,
                            i32::from(row.minimum),
                            i32::from(row.maximum),
                            i32::from(row.placed),
                        );
                    }
                }
                _ => {}
            }
        }
        let n = data.rows.len().min(ROWS);
        if counts_differ(dialog, n, &self.sent) {
            self.update(dialog, data);
            let now = counts(dialog, n);
            self.sent = now.clone();
            let actions = &mut frame.out.actions;
            let at = actions
                .iter()
                .position(|a| *a == Action::Ok)
                .unwrap_or(actions.len());
            actions.insert(at, Action::SetTradeCounts(now));
        }
    }
}
