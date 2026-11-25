use std::collections::HashMap;

use crate::dialog::Rect;
use mandrake_common::{FourCC, M2Error};

pub const FONT_DEFINITION_SIZE: usize = 0x40;
pub const GLYPH_HEADER_SIZE: usize = 16;
pub const FONT_STYLE_COUNT: usize = 4;
pub const STYLE_ITALIC: i16 = 0;
pub const STYLE_BOLD: i16 = 1;
pub const STYLE_PLAIN: i16 = -1;

pub const TEXT_LOCALE_ID: FourCC = FourCC(*b"in#1");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glyph {
    pub code: u16,
    pub advance: i16,
    pub width: i16,
    pub height: i16,
    pub origin_x: i16,
    pub origin_y: i16,
    pub coverage: Vec<u8>,
}

fn builtin_space() -> Glyph {
    Glyph {
        code: 0x20,
        advance: 4,
        width: 0,
        height: 0,
        origin_x: 0,
        origin_y: 0,
        coverage: Vec::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontTag {
    pub flags: u32,
    pub ascending: i16,
    pub descending: i16,
    pub leading: i16,
    pub style_fonts: [Option<FourCC>; FONT_STYLE_COUNT],
    pub glyphs: Vec<Glyph>,
    index: HashMap<u16, usize>,
}

fn be16(d: &[u8], o: usize) -> i16 {
    i16::from_be_bytes([d[o], d[o + 1]])
}

fn be32(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

impl FontTag {
    pub fn from_bytes(d: &[u8]) -> Result<Self, M2Error> {
        if d.len() < FONT_DEFINITION_SIZE {
            return Err(M2Error::TagParse(
                "font: shorter than its definition".into(),
            ));
        }
        let count = be32(d, 0x0c) as i32;
        let offset = be32(d, 0x10) as usize;
        let size = be32(d, 0x14) as usize;
        let data = d
            .get(offset..offset.saturating_add(size))
            .ok_or_else(|| M2Error::TagParse("font: glyph data past the tag".into()))?;
        let style_fonts = std::array::from_fn(|i| {
            let b: [u8; 4] = d[0x18 + 4 * i..0x1c + 4 * i].try_into().unwrap();
            (b != [0xff; 4]).then_some(FourCC(b))
        });
        let mut glyphs = Vec::new();
        let mut index = HashMap::new();
        let mut o = 0usize;
        for _ in 0..count.max(0) {
            let hdr_end = o
                .checked_add(GLYPH_HEADER_SIZE)
                .filter(|&e| e <= data.len())
                .ok_or_else(|| {
                    M2Error::TagParse(format!("font: glyph {} header past the data", glyphs.len()))
                })?;
            let (w, h) = (be16(data, o + 4), be16(data, o + 6));
            if w < 0 || h < 0 {
                return Err(M2Error::TagParse(format!(
                    "font: glyph {} has negative dimensions {w}x{h}",
                    glyphs.len()
                )));
            }
            let n = (w as usize)
                .checked_mul(h as usize)
                .ok_or_else(|| M2Error::TagParse("font: glyph size overflow".into()))?;
            let px_end = hdr_end
                .checked_add(n)
                .ok_or_else(|| M2Error::TagParse("font: glyph pixels overflow".into()))?;
            let px = data
                .get(hdr_end..px_end)
                .ok_or_else(|| M2Error::TagParse("font: glyph pixels past the data".into()))?;
            let g = Glyph {
                code: be16(data, o) as u16,
                advance: be16(data, o + 2),
                width: w,
                height: h,
                origin_x: be16(data, o + 8),
                origin_y: be16(data, o + 10),
                coverage: px.to_vec(),
            };
            index.entry(g.code).or_insert(glyphs.len());
            glyphs.push(g);
            o = px_end
                .checked_add(n & 1)
                .ok_or_else(|| M2Error::TagParse("font: glyph offset overflow".into()))?;
        }
        let page0 = glyphs.iter().any(|g| g.code >> 8 == 0);
        if page0 && !index.contains_key(&0x20) {
            index.insert(0x20, glyphs.len());
            glyphs.push(builtin_space());
        }
        Ok(Self {
            flags: be32(d, 0),
            ascending: be16(d, 4),
            descending: be16(d, 6),
            leading: be16(d, 8),
            style_fonts,
            glyphs,
            index,
        })
    }

    pub fn glyph(&self, code: u16) -> Option<&Glyph> {
        self.index.get(&code).map(|&i| &self.glyphs[i])
    }

    pub fn margin(&self) -> i32 {
        (self.ascending as i32 + self.descending as i32) / 5
    }

    pub fn line_height(&self) -> i32 {
        self.ascending as i32 + self.descending as i32
    }

    pub fn line_pitch(&self) -> i32 {
        self.ascending as i32 + self.leading as i32 + self.descending as i32
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFamily {
    pub id: FourCC,
    pub base: FontTag,
    pub styles: [Option<FontTag>; FONT_STYLE_COUNT],
}

impl FontFamily {
    pub fn load(id: FourCC, tag: impl Fn(FourCC) -> Option<Vec<u8>>) -> Result<Self, M2Error> {
        let raw = tag(id).ok_or_else(|| M2Error::TagParse(format!("font {id}: missing")))?;
        let base = FontTag::from_bytes(&raw)?;
        let styles = std::array::from_fn(|i| {
            base.style_fonts[i]
                .and_then(&tag)
                .and_then(|d| FontTag::from_bytes(&d).ok())
        });
        Ok(Self { id, base, styles })
    }

    pub fn from_font(id: FourCC, base: FontTag) -> Self {
        Self {
            id,
            base,
            styles: [None, None, None, None],
        }
    }

    pub fn font(&self, style: i16) -> &FontTag {
        if style < 0 {
            return &self.base;
        }
        self.styles
            .get(style as usize)
            .and_then(Option::as_ref)
            .unwrap_or(&self.base)
    }

    pub fn glyph(&self, style: i16, code: u16) -> Option<&Glyph> {
        self.font(style).glyph(code)
    }

    pub fn adjust_line_height(&mut self, delta: i16) {
        self.base.leading += delta;
        for f in self.styles.iter_mut().flatten() {
            f.leading += delta;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextLocale {
    pub language: i16,
    pub truncation: Vec<u8>,
    pub no_truncation_after: Vec<u8>,
    pub word_end: Vec<u8>,
    pub no_word_end: Vec<u8>,
    pub no_word_begin: Vec<u8>,
}

impl TextLocale {
    pub fn from_stli_bytes(d: &[u8]) -> Self {
        let s: Vec<&[u8]> = d.split(|&b| b == b'\r').collect();
        let get = |i: usize| s.get(i).map_or_else(Vec::new, |x| x.to_vec());
        let language = std::str::from_utf8(s.first().copied().unwrap_or(b"0"))
            .ok()
            .and_then(|t| {
                let t = t.trim_start();
                let end = t
                    .find(|c: char| !(c.is_ascii_digit() || c == '-' || c == '+'))
                    .unwrap_or(t.len());
                t[..end].parse::<i64>().ok()
            })
            .unwrap_or(0)
            .clamp(0, 5) as i16;
        Self {
            language,
            truncation: get(1),
            no_truncation_after: get(2),
            word_end: get(3),
            no_word_end: get(4),
            no_word_begin: get(5),
        }
    }
}

fn byte_at(t: &[u8], i: usize) -> u8 {
    t.get(i).copied().unwrap_or(0)
}

fn read_code(t: &[u8], i: usize) -> (u16, usize) {
    let (a, b) = (byte_at(t, i), byte_at(t, i + 1));
    if a != 0 && b != 0 && a == b'|' && b != b'\r' {
        if b == b'|' {
            (b'|' as u16, 2)
        } else {
            (u16::from_be_bytes([a, b]), 2)
        }
    } else {
        (a as u16, 1)
    }
}

fn in_set(set: &[u8], code: u16) -> bool {
    let mut i = 0;
    loop {
        let (c, n) = read_code(set, i);
        if c == 0 {
            return false;
        }
        if c == code {
            return true;
        }
        i += n;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharClass {
    End,
    Return,
    Break,
    Char,
}

#[derive(Debug, Clone)]
pub struct TextCursor<'a> {
    pub text: &'a [u8],
    pub index: usize,
    pub style: i16,
    pub underline: bool,
    pub colour: i16,
    pub link: i32,
    pub class: CharClass,
    pub ch: u16,
}

impl<'a> TextCursor<'a> {
    pub fn new(text: &'a [u8], style: i16) -> Self {
        Self {
            text,
            index: 0,
            style,
            underline: false,
            colour: -1,
            link: -1,
            class: CharClass::End,
            ch: 0,
        }
    }

    pub fn advance(&mut self, locale: &TextLocale) -> CharClass {
        let t = self.text;
        let ch = loop {
            let (a, b) = (byte_at(t, self.index), byte_at(t, self.index + 1));
            if a != 0 && b != 0 && a == b'|' && b != b'\r' {
                self.index += 2;
                if b == b'|' {
                    break b'|' as u16;
                }
                match b {
                    b'0'..=b'9' => self.colour = (b - b'0') as i16,
                    b'A'..=b'Z' => self.colour = b as i16 - 0x37,
                    b'p' => {
                        self.style = STYLE_PLAIN;
                        self.underline = false;
                        self.colour = -1;
                        self.link = -1;
                    }
                    b'i' => self.style = STYLE_ITALIC,
                    b'b' => self.style = STYLE_BOLD,
                    b'u' => self.underline = true,
                    b'l' => {
                        self.link = self.index as i32;
                        self.colour = 5;
                    }
                    b'x' => self.colour = -2,
                    _ => {}
                }
                continue;
            }
            self.index += 1;
            break a as u16;
        };
        self.ch = ch;
        self.class = match ch {
            0 => CharClass::End,
            0x0d => CharClass::Return,
            _ => {
                let (next, _) = read_code(t, self.index);
                if !in_set(&locale.word_end, ch) || in_set(&locale.no_word_begin, next) {
                    CharClass::Char
                } else {
                    CharClass::Break
                }
            }
        };
        self.class
    }
}

pub fn string_width(family: &FontFamily, style: i16, text: &[u8], locale: &TextLocale) -> i32 {
    let mut c = TextCursor::new(text, style);
    let mut w: i16 = 0;
    while matches!(c.advance(locale), CharClass::Break | CharClass::Char) {
        if let Some(g) = family.glyph(c.style, c.ch) {
            w = w.wrapping_add(g.advance);
        }
    }
    w as i32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnd {
    Line,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrappedLine {
    pub style: i16,
    pub count: usize,
    pub width: i32,
    pub end: LineEnd,
}

pub const TEXT_WRAP: u32 = 0x10;

pub fn wrap_string(
    family: &FontFamily,
    style: i16,
    text: &[u8],
    max_width: i32,
    flags: u32,
    locale: &TextLocale,
) -> WrappedLine {
    let wrap = flags & TEXT_WRAP != 0;
    let mut out = WrappedLine {
        style,
        count: 0,
        width: 0,
        end: LineEnd::Line,
    };
    let mut c = TextCursor::new(text, style);
    let mut prev: Option<CharClass> = None;
    let mut break_count = 0usize;
    let mut break_width = 0i32;
    loop {
        let class = c.advance(locale);
        out.style = c.style;
        match class {
            CharClass::End => {
                out.end = LineEnd::End;
                out.count = c.index;
                return out;
            }
            CharClass::Return => {
                out.end = LineEnd::Line;
                out.count = c.index;
                return out;
            }
            _ => {}
        }
        if let Some(g) = family.glyph(c.style, c.ch) {
            if prev == Some(CharClass::Break) && class != CharClass::Break {
                break_width = out.width;
                break_count = out.count;
            }
            if out.width + g.width as i32 >= max_width {
                if wrap {
                    if break_count > 0 {
                        out.count = break_count;
                        out.width = break_width;
                    } else {
                        out.count = c.index;
                    }
                    out.end = LineEnd::Line;
                    return out;
                }
            } else {
                out.width = (out.width as i16).wrapping_add(g.advance) as i32;
            }
        }
        out.count = c.index;
        prev = Some(class);
    }
}

pub fn transparency_table_entry(level: u32, a: u32, b: u32) -> u32 {
    (a * level + b * (15 - level) + 7) / 15
}

pub const TEXT_COLOR_CODES: [u32; 36] = [
    0xffffff, 0xcbcbcb, 0x686868, 0x000000, 0x0000ff, 0x84aaff, 0x1f72e7, 0x023e9d, 0x4b0082,
    0xb29fff, 0x8353ff, 0x5203cc, 0xff0000, 0xfe8a7b, 0xda3220, 0x871810, 0xfcfc38, 0xffd700,
    0xdba30d, 0x9b7000, 0xff8000, 0xdb740d, 0x7f4000, 0x00ff00, 0x99f869, 0x4bc315, 0x008000,
    0x006400, 0x00ffff, 0x7ef3e9, 0x07bfbc, 0x008583, 0x00504e, 0xff9aff, 0xff34ff, 0x8b008b,
];

pub type RgbColor = [u16; 3];

pub const SHADOW_COLOR: RgbColor = [0, 0, 0];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap32 {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u32>,
}

impl Bitmap32 {
    pub fn new(width: i32, height: i32, fill: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![fill; (width.max(0) * height.max(0)) as usize],
        }
    }

    pub fn get(&self, x: i32, y: i32) -> u32 {
        self.pixels[(y * self.width + x) as usize]
    }

    pub fn set(&mut self, x: i32, y: i32, v: u32) {
        self.pixels[(y * self.width + x) as usize] = v;
    }

    pub fn to_rgba(&self) -> Vec<u8> {
        self.pixels
            .iter()
            .flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8, 255])
            .collect()
    }
}

pub mod text_flags {
    pub const CENTER: u32 = 0x01;
    pub const MIDDLE: u32 = 0x02;
    pub const RIGHT: u32 = 0x04;
    pub const BOTTOM: u32 = 0x08;
    pub const WRAP: u32 = 0x10;
    pub const SHADOW: u32 = 0x20;
    pub const COLOR_CODES: u32 = 0x40;
    pub const CLIP_LINES: u32 = 0x100;
    pub const MULTILINE: u32 = 0x200;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextClip {
    None,
    SameAsRect,
    Rect(Rect),
}

fn blend_pixel(dst: u32, src: u32, level: u32) -> u32 {
    let t = |d: u32, s: u32| transparency_table_entry(level, d & 31, s & 31);
    0xff00_0000
        | t(dst >> 19, src >> 19) << 19
        | t(dst >> 11, src >> 11) << 11
        | t(dst >> 3, src >> 3) << 3
}

trait GlyphTarget {
    fn size(&self) -> (i32, i32);
    fn plot(&mut self, x: i32, y: i32, coverage: u8, src: u32, t: u32);
}

impl GlyphTarget for Bitmap32 {
    fn size(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    fn plot(&mut self, x: i32, y: i32, c: u8, src: u32, t: u32) {
        if t == 0 {
            match c {
                0 => self.set(x, y, src),
                0xff => {}
                _ => {
                    let d = self.get(x, y);
                    self.set(x, y, blend_pixel(d, src, (c >> 4) as u32));
                }
            }
        } else if c != 0xff {
            let level = 15 - (((15 - (c >> 4) as u32) * (15 - t)) >> 4);
            let d = self.get(x, y);
            self.set(x, y, blend_pixel(d, src, level));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMask {
    pub width: i32,
    pub height: i32,
    pub levels: Vec<u8>,
}

impl TextMask {
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            levels: vec![0xff; (width.max(0) * height.max(0)) as usize],
        }
    }

    pub fn row(&self, y: i32) -> &[u8] {
        let from = (y * self.width) as usize;
        &self.levels[from..from + self.width as usize]
    }
}

impl GlyphTarget for TextMask {
    fn size(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    fn plot(&mut self, x: i32, y: i32, c: u8, _src: u32, _t: u32) {
        if c != 0xff {
            let level = 0xff - (((0xff - c as u32) * 0xffff) >> 16);
            let pixel = &mut self.levels[(y * self.width + x) as usize];
            *pixel = (*pixel).min(level as u8);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_line(
    bitmap: &mut impl GlyphTarget,
    pen: &mut (i32, i32),
    rect: Option<Rect>,
    clip: Option<Rect>,
    family: &FontFamily,
    style: i16,
    color: RgbColor,
    text: &[u8],
    count: usize,
    transparency: u32,
    color_codes: bool,
    locale: &TextLocale,
) {
    let (width, height) = bitmap.size();
    let (mut x0, mut x1, mut y0, mut y1) = (0, width, 0, height);
    if let Some(r) = rect {
        x0 = r.left.max(0);
        x1 = width.min(r.right);
        y0 = r.top.max(0);
        y1 = height.min(r.bottom);
    }
    if let Some(c) = clip {
        x0 = x0.max(c.left);
        x1 = x1.min(c.right);
        y0 = y0.max(c.top);
        y1 = y1.min(c.bottom);
    }
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let transparency = transparency.min(0xffff);
    let t = transparency >> 12;
    let rgb = |c: RgbColor| -> u32 {
        (((0xffff - transparency) << 16) & 0xff00_0000)
            | ((c[0] as u32) << 8) & 0xff_0000
            | (c[1] as u32 & 0xff00)
            | (c[2] as u32 >> 8)
    };
    let mut src = rgb(color);
    let mut current_code: Option<i16> = None;
    let mut cur = TextCursor::new(text, style);
    while cur.index < count {
        cur.advance(locale);
        let Some(g) = family.glyph(cur.style, cur.ch) else {
            continue;
        };
        if cur.colour == -2 {
            pen.0 += g.advance as i32;
            continue;
        }
        let code = if color_codes { cur.colour } else { -1 };
        if current_code != Some(code) {
            src = if code >= 0 {
                let v = TEXT_COLOR_CODES[code as usize];
                rgb([
                    ((v >> 16) as u16 & 0xff) << 8,
                    ((v >> 8) as u16 & 0xff) << 8,
                    (v as u16 & 0xff) << 8,
                ])
            } else {
                rgb(color)
            };
            current_code = Some(code);
        }
        let mut gx = pen.0 - g.origin_x as i32;
        let mut gy = pen.1 - g.origin_y as i32;
        let mut w = if gx + g.width as i32 > x1 {
            x1 - gx
        } else {
            g.width as i32
        };
        let mut skip_x = 0;
        if x0 > gx {
            skip_x = x0 - gx;
            gx = x0;
            w -= skip_x;
        }
        let mut h = if gy + g.height as i32 > y1 {
            y1 - gy
        } else {
            g.height as i32
        };
        let mut skip_y = 0;
        if y0 > gy {
            skip_y = y0 - gy;
            gy = y0;
            h -= skip_y;
        }
        for row in 0..h.max(0) {
            for col in 0..w.max(0) {
                let c = g.coverage[((row + skip_y) * g.width as i32 + skip_x + col) as usize];
                bitmap.plot(gx + col, gy + row, c, src, t);
            }
        }
        pen.0 += g.advance as i32;
    }
}

fn truncate(
    family: &FontFamily,
    style: i16,
    text: &[u8],
    avail: i32,
    locale: &TextLocale,
) -> Option<Vec<u8>> {
    let len = text.iter().position(|&b| b == 0).unwrap_or(text.len());
    let text = &text[..len];
    let mut ellipsis = locale.truncation.clone();
    if len == 0 {
        return None;
    }
    if text[len - 1] == b'\r' {
        ellipsis.push(b'\r');
    }
    if ellipsis.len() + len > 0xff {
        return None;
    }
    let mut buf = text.to_vec();
    buf.push(0);
    let cut = |buf: &mut Vec<u8>, at: usize, with: &[u8]| {
        buf.truncate(at);
        buf.extend_from_slice(with);
        buf.push(0);
    };
    let last_char = |buf: &[u8], end: usize| -> (usize, u16) {
        let mut i = 0;
        loop {
            let (c, n) = read_code(buf, i);
            if end > i + n {
                i += n;
            } else {
                return (i, c);
            }
        }
    };
    let mut end = len;
    loop {
        if string_width(family, style, &buf, locale) < avail {
            return Some(buf[..buf.iter().position(|&b| b == 0).unwrap()].to_vec());
        }
        let (i, _) = last_char(&buf, end);
        if (buf[i] as char).is_ascii_whitespace() || buf[i] == 0x0b {
            buf[i] = 0;
            end = i;
        } else if i == 0 {
            cut(&mut buf, 0, &ellipsis);
            end = 0;
        } else {
            end = i;
            loop {
                let (j, c) = last_char(&buf, end);
                if in_set(&locale.no_truncation_after, c) {
                    end = j;
                    if j > 0 {
                        continue;
                    }
                    cut(&mut buf, j, &ellipsis);
                } else {
                    cut(&mut buf, end, &ellipsis);
                }
                break;
            }
        }
        if end == 0 {
            return None;
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_string(
    bitmap: &mut Bitmap32,
    rect: Rect,
    clip: TextClip,
    family: &FontFamily,
    style: i16,
    text: &[u8],
    color: RgbColor,
    flags: u32,
    transparency: u32,
    locale: &TextLocale,
) -> (i32, i32) {
    draw_string_at(
        bitmap,
        (0, 0),
        rect,
        clip,
        family,
        style,
        text,
        color,
        flags,
        transparency,
        locale,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn draw_string_at(
    bitmap: &mut Bitmap32,
    pen_in: (i32, i32),
    rect: Rect,
    clip: TextClip,
    family: &FontFamily,
    style: i16,
    text: &[u8],
    color: RgbColor,
    flags: u32,
    transparency: u32,
    locale: &TextLocale,
) -> (i32, i32) {
    draw_string_into(
        bitmap,
        pen_in,
        rect,
        clip,
        family,
        style,
        text,
        color,
        flags,
        transparency,
        locale,
    )
}

pub fn string_height(
    family: &FontFamily,
    style: i16,
    text: &[u8],
    width: i32,
    locale: &TextLocale,
) -> i32 {
    let font = &family.base;
    let first = font.ascending as i32 + font.descending as i32;
    let mut height = first;
    let mut style = style;
    let mut offset = 0usize;
    loop {
        let rest = text.get(offset..).unwrap_or(&[]);
        let wl = wrap_string(
            family,
            style,
            rest,
            width - font.margin(),
            text_flags::WRAP,
            locale,
        );
        if wl.end == LineEnd::End {
            return height;
        }
        height += font.line_pitch();
        style = wl.style;
        offset += wl.count;
    }
}

pub fn draw_string_to_mask(
    mask: &mut TextMask,
    family: &FontFamily,
    style: i16,
    text: &[u8],
    locale: &TextLocale,
) -> (i32, i32) {
    let rect = Rect::new(0, 0, mask.height, mask.width);
    draw_string_into(
        mask,
        (0, 0),
        rect,
        TextClip::None,
        family,
        style,
        text,
        [0xffff; 3],
        text_flags::WRAP,
        0,
        locale,
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_string_into(
    bitmap: &mut impl GlyphTarget,
    pen_in: (i32, i32),
    rect: Rect,
    clip: TextClip,
    family: &FontFamily,
    style: i16,
    text: &[u8],
    color: RgbColor,
    flags: u32,
    transparency: u32,
    locale: &TextLocale,
) -> (i32, i32) {
    let font = &family.base;
    let (asc, desc, margin) = (font.ascending as i32, font.descending as i32, font.margin());
    let owned;
    let mut text = text;
    if flags & (text_flags::WRAP | text_flags::MULTILINE) == 0 {
        let avail = (rect.right - margin - rect.left) as i16 as i32;
        if let Some(t) = truncate(family, style, text, avail, locale) {
            owned = t;
            text = &owned;
        }
    }
    let mut offset = 0usize;
    let mut line = 0;
    let mut style = style;
    let mut start = (0, 0);
    let mut pen;
    loop {
        pen = (rect.left + margin, 0);
        pen.1 = if flags & text_flags::BOTTOM == 0 {
            rect.top + asc + line * font.line_pitch()
        } else {
            rect.bottom - desc - line * font.line_pitch()
        };
        if line == 0 {
            start = pen;
            pen.0 += pen_in.0;
        }
        pen.1 += pen_in.1;
        let rest = text.get(offset..).unwrap_or(&[]);
        let wl = wrap_string(family, style, rest, rect.right - pen.0, flags, locale);
        match flags & 5 {
            1 => pen.0 = rect.left + ((rect.right - rect.left - wl.width) >> 1),
            4 => pen.0 = rect.right - margin - wl.width,
            _ => {}
        }
        if flags & text_flags::MIDDLE != 0 {
            pen.1 = rect.top + asc + ((rect.bottom - rect.top - (asc + desc)) >> 1);
        }
        if flags & text_flags::CLIP_LINES == 0 || pen.1 < rect.bottom {
            let clip_of = |r: Rect| match clip {
                TextClip::None => None,
                TextClip::SameAsRect => Some(r),
                TextClip::Rect(c) => Some(c),
            };
            if flags & text_flags::SHADOW != 0 {
                let r = rect.offset(1, 1);
                let mut p = (pen.0 + 1, pen.1 + 1);
                draw_line(
                    bitmap,
                    &mut p,
                    Some(r),
                    clip_of(r),
                    family,
                    style,
                    SHADOW_COLOR,
                    rest,
                    wl.count,
                    transparency,
                    false,
                    locale,
                );
            }
            draw_line(
                bitmap,
                &mut pen,
                Some(rect),
                clip_of(rect),
                family,
                style,
                color,
                rest,
                wl.count,
                transparency,
                flags & text_flags::COLOR_CODES != 0,
                locale,
            );
        }
        offset += wl.count;
        line += 1;
        style = wl.style;
        if wl.end == LineEnd::End {
            break;
        }
    }
    (pen.0 - start.0, pen.1 - start.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locale() -> TextLocale {
        TextLocale::from_stli_bytes(
            b"0\r\xc9\r `~!@#$%^&*+-=|\\\";\xd3\xd2\xd4\xd5',.\xc9\r ;,:./-\r($[\xb4{\r!%),.?]}\r0\r",
        )
    }

    fn font(asc: i16, desc: i16, glyphs: &[(u8, i16, i16, i16, i16, i16, u8)]) -> Vec<u8> {
        let mut data = Vec::new();
        for &(c, adv, w, h, ox, oy, fill) in glyphs {
            for v in [c as i16, adv, w, h, ox, oy, 0, 0] {
                data.extend_from_slice(&v.to_be_bytes());
            }
            let n = (w * h) as usize;
            data.extend(std::iter::repeat_n(fill, n));
            if n & 1 == 1 {
                data.push(0);
            }
        }
        let mut d = vec![0u8; FONT_DEFINITION_SIZE];
        d[4..6].copy_from_slice(&asc.to_be_bytes());
        d[6..8].copy_from_slice(&desc.to_be_bytes());
        d[0x0c..0x10].copy_from_slice(&(glyphs.len() as u32).to_be_bytes());
        d[0x10..0x14].copy_from_slice(&0x40u32.to_be_bytes());
        d[0x14..0x18].copy_from_slice(&(data.len() as u32).to_be_bytes());
        d[0x18..0x28].fill(0xff);
        d.extend(data);
        d
    }

    #[test]
    fn parses_glyphs_and_adds_the_space() {
        let f = FontTag::from_bytes(&font(10, 3, &[(b'a', 6, 3, 3, 0, 3, 0)])).unwrap();
        assert_eq!(f.margin(), 2);
        assert_eq!(f.line_height(), 13);
        let a = f.glyph(b'a' as u16).unwrap();
        assert_eq!((a.advance, a.width, a.coverage.len()), (6, 3, 9));
        assert_eq!(f.glyph(0x20).unwrap().advance, 4);
        assert!(f.glyph(b'b' as u16).is_none());
    }

    #[test]
    fn cursor_reads_style_codes() {
        let loc = locale();
        let mut c = TextCursor::new(b"a|bB||c|p d", -1);
        assert_eq!(c.advance(&loc), CharClass::Char);
        assert_eq!((c.ch, c.style), (b'a' as u16, -1));
        c.advance(&loc);
        assert_eq!((c.ch, c.style), (b'B' as u16, STYLE_BOLD));
        c.advance(&loc);
        assert_eq!(c.ch, b'|' as u16);
        c.advance(&loc);
        assert_eq!(c.ch, b'c' as u16);
        assert_eq!(c.advance(&loc), CharClass::Break);
        assert_eq!(c.style, STYLE_PLAIN);
        c.advance(&loc);
        assert_eq!(c.advance(&loc), CharClass::End);
        assert_eq!(c.index, 12);
        let mut c = TextCursor::new(b"a ,", -1);
        c.advance(&loc);
        assert_eq!(c.advance(&loc), CharClass::Char);
        let mut c = TextCursor::new(b"|3x|xy", -1);
        c.advance(&loc);
        assert_eq!(c.colour, 3);
        c.advance(&loc);
        assert_eq!(c.colour, -2);
    }

    #[test]
    fn wraps_at_the_last_break() {
        let loc = locale();
        let fam = FontFamily::from_font(
            FourCC(*b"test"),
            FontTag::from_bytes(&font(
                8,
                2,
                &[(b'a', 5, 4, 1, 0, 1, 0), (b' ', 3, 0, 0, 0, 0, 0)],
            ))
            .unwrap(),
        );
        assert_eq!(string_width(&fam, -1, b"aa a\raaa", &loc), 18);
        let l = wrap_string(&fam, -1, b"aa aa", 20, TEXT_WRAP, &loc);
        assert_eq!((l.count, l.width, l.end), (3, 13, LineEnd::Line));
        let l = wrap_string(&fam, -1, b"aa aa", 20, 0, &loc);
        assert_eq!((l.count, l.end), (6, LineEnd::End));
        let l = wrap_string(&fam, -1, b"aaaaa", 12, TEXT_WRAP, &loc);
        assert_eq!(l.count, 3);
    }

    #[test]
    fn transparency_table_matches_the_engine_corners() {
        assert_eq!(transparency_table_entry(15, 31, 0), 31);
        assert_eq!(transparency_table_entry(0, 31, 7), 7);
        assert_eq!(transparency_table_entry(8, 0, 31), 14);
        assert_eq!(transparency_table_entry(8, 0, 2), 1);
    }

    #[test]
    fn draws_coverage_shadow_and_truncates() {
        let loc = locale();
        let mut f = font(4, 1, &[(b'a', 3, 2, 1, 0, 1, 0), (0xc9, 2, 1, 1, 0, 1, 0)]);
        let a_px = 0x40 + 16 + 1;
        f[a_px] = 0x7f;
        let fam = FontFamily::from_font(FourCC(*b"test"), FontTag::from_bytes(&f).unwrap());
        let mut bmp = Bitmap32::new(16, 8, 0xff40_4040);
        let rect = Rect::new(0, 0, 8, 16);
        draw_string(
            &mut bmp,
            rect,
            TextClip::SameAsRect,
            &fam,
            -1,
            b"a",
            [0xffff, 0xffff, 0xffff],
            text_flags::SHADOW,
            0,
            &loc,
        );
        assert_eq!(bmp.get(1, 3), 0xffff_ffff);
        let grey = 0x40 >> 3;
        let mix = transparency_table_entry(7, grey, 31) << 3;
        assert_eq!(bmp.get(2, 3), 0xff00_0000 | mix << 16 | mix << 8 | mix);
        assert_eq!(bmp.get(2, 4), 0xff00_0000);
        let sh = transparency_table_entry(7, grey, 0) << 3;
        assert_eq!(bmp.get(3, 4) & 0xff, sh);
        assert_eq!(
            truncate(&fam, -1, b"aaaa", 15, &loc),
            Some(b"aaaa".to_vec())
        );
        assert_eq!(
            truncate(&fam, -1, b"aaaaa", 15, &loc),
            Some(b"aaaa\xc9".to_vec())
        );
    }
}
