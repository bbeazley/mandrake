pub use mandrake_sim::game_speed::*;

use mandrake_common::FourCC;
use mandrake_tag::Rect;
use mandrake_tag::font::{self, Bitmap32, TextClip, text_flags};

use crate::command_bar::{self, PromptTags};
use crate::unit_info::string_from_list;

pub const STRING_LIST: FourCC = FourCC(*b"inrs");
pub const POPUP_LIFE: i32 = 0xf;
pub const POPUP_FADE: i32 = 7;
const POPUP_HEIGHT: i32 = 0x16;
const POPUP_EXTRA_WIDTH: i32 = 0x32;
const POPUP_GAP: i32 = 0x14;
const POPUP_BAND_TRANSPARENCY: u32 = 0x8000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeedPopup {
    pub title: Vec<u8>,
    pub text: Vec<u8>,
    pub stamp: i32,
}

impl SpeedPopup {
    pub fn new(list: Option<&[Vec<u8>]>, index: i16, stamp: i32) -> Self {
        let text = string_from_list(list, i32::from(index) + 1);
        let text = if text.is_empty() {
            format!("{}x", GAME_SPEEDS[index as usize] >> 8).into_bytes()
        } else {
            text.to_vec()
        };
        Self {
            title: string_from_list(list, 0).to_vec(),
            text,
            stamp,
        }
    }

    pub fn transparency(&self, now: i32) -> Option<(u16, u16)> {
        let past = now - self.stamp - POPUP_LIFE;
        if past < 1 {
            return Some((POPUP_BAND_TRANSPARENCY as u16, 0));
        }
        if past >= POPUP_FADE {
            return None;
        }
        let fade = ((past << 16) / POPUP_FADE) as u32;
        Some(((POPUP_BAND_TRANSPARENCY + fade / 2) as u16, fade as u16))
    }

    fn width(&self, tags: &PromptTags) -> i32 {
        let width = |s: &[u8]| font::string_width(&tags.family, font::STYLE_PLAIN, s, &tags.locale);
        let title = width(&self.title);
        let average = if self.title.is_empty() {
            0
        } else {
            title / self.title.len() as i32
        };
        title + average + POPUP_EXTRA_WIDTH + width(&self.text)
    }

    pub fn band(&self, tags: &PromptTags, frame: Rect) -> Rect {
        let width = self.width(tags);
        let x = frame.left + (frame.right - frame.left - width) / 2;
        let y = frame.top + (frame.bottom - frame.top - POPUP_HEIGHT) / 3;
        Rect::new(0, 0, POPUP_HEIGHT, width).offset(x, y)
    }

    fn draw(&self, tags: &PromptTags, band: Rect, background: u32) -> Bitmap32 {
        let (w, h) = (band.right - band.left, band.bottom - band.top);
        let mut bmp = Bitmap32::new(w, h, background);
        let mut write = |rect: Rect, text: &[u8], flags: u32| {
            font::draw_string(
                &mut bmp,
                rect,
                TextClip::None,
                &tags.family,
                font::STYLE_PLAIN,
                text,
                tags.color,
                flags,
                0,
                &tags.locale,
            )
        };
        let (pen, _) = write(Rect::new(0, 0, h, w), &self.title, text_flags::MIDDLE);
        write(
            Rect::new(0, pen + POPUP_GAP, h, w),
            &self.text,
            text_flags::MIDDLE | text_flags::RIGHT,
        );
        bmp
    }

    pub fn rgba(&self, tags: &PromptTags, band: Rect) -> (u32, u32, Vec<u8>) {
        let black = self.draw(tags, band, 0xff00_0000);
        let white = self.draw(tags, band, 0xffff_ffff);
        (
            black.width as u32,
            black.height as u32,
            command_bar::coverage_rgba(&black, &white),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_popup_uses_the_list_then_the_multiple() {
        let list: Vec<Vec<u8>> = [
            "Game Speed",
            "Stopped",
            "1/16th",
            "1/8th",
            "1/4th",
            "Half",
            "Normal",
            "Double",
            "4x",
            "8x",
            "16x",
        ]
        .iter()
        .map(|s| s.as_bytes().to_vec())
        .collect();
        let text = |i| SpeedPopup::new(Some(&list), i, 0).text;
        assert_eq!(text(5), b"Normal");
        assert_eq!(text(9), b"16x");
        assert_eq!(text(10), b"32x");
        assert_eq!(text(11), b"64x");
        assert_eq!(SpeedPopup::new(Some(&list), 0, 0).title, b"Game Speed");
    }

    #[test]
    fn the_popup_holds_for_its_life_then_fades() {
        let p = SpeedPopup::new(None, 6, 100);
        assert_eq!(p.transparency(100), Some((0x8000, 0)));
        assert_eq!(p.transparency(115), Some((0x8000, 0)));
        let (band, text) = p.transparency(116).unwrap();
        assert!(band > 0x8000 && text > 0);
        assert_eq!(p.transparency(122), None);
    }
}
