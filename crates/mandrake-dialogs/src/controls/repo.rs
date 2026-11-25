use mandrake_session::{EpiloguePage, ReportSection, SectionKind};
use mandrake_tag::font::{self, Bitmap32, RgbColor, TextClip, text_flags};
use mandrake_tag::{DialogItem, Rect};

use super::{ControlProc, DrawContext, ItemState};
use crate::resources::Resources;

const WHITE: RgbColor = [0xffff; 3];
const HEADING_FONT: usize = 3;
const TABLE_FONT: usize = 0;
const HEADING_WIDTH: i32 = 0x250;
const HEADING_HEIGHT: i32 = 0x19;
const ROW_HEIGHT: i32 = 0x0e;
const SECTION_GAP: i32 = 10;
const ENCODED_LIMIT: usize = 0x400;

const fn right_edges(kind: SectionKind) -> &'static [i32] {
    match kind {
        SectionKind::Survivors => &[150, 200, 250, 330],
        SectionKind::Heroes => &[230, 280, 330],
        SectionKind::Kills => &[150, 200],
        SectionKind::DamageRatio | SectionKind::KillRatio { .. } => &[],
    }
}

struct Placed {
    columns: &'static [i32],
    rows: i32,
}

struct Encoded {
    sections: Vec<Placed>,
    strings: Vec<Vec<u8>>,
}

impl Encoded {
    fn new(page: &EpiloguePage) -> Self {
        let mut encoder = Encoder::default();
        encoder.push(&page.title.0);
        let mut sections = vec![Placed {
            columns: &[],
            rows: 0,
        }];
        for section in &page.sections {
            sections.push(encoder.section(section));
        }
        Self {
            sections,
            strings: encoder.strings,
        }
    }
}

#[derive(Default)]
struct Encoder {
    strings: Vec<Vec<u8>>,
    used: usize,
}

impl Encoder {
    fn push(&mut self, text: &[u8]) {
        let size = text.len() + 1;
        if self.used + size < ENCODED_LIMIT {
            self.used += size;
            self.strings.push(text.to_vec());
        }
    }

    fn section(&mut self, section: &ReportSection) -> Placed {
        self.push(&section.heading.0);
        for cell in section.columns.iter().chain(section.rows.iter().flatten()) {
            self.push(&cell.0);
        }
        let header = i32::from(!section.columns.is_empty());
        Placed {
            columns: right_edges(section.kind),
            rows: header + section.rows.len() as i32,
        }
    }
}

pub struct Report {
    heading_color: RgbColor,
    header_color: RgbColor,
    row_color: RgbColor,
    page: Option<Encoded>,
}

impl Report {
    pub fn new(_item: &DialogItem, _res: &mut Resources) -> Self {
        Self {
            heading_color: WHITE,
            header_color: WHITE,
            row_color: WHITE,
            page: None,
        }
    }

    pub fn set_page(&mut self, item: &mut ItemState, page: Option<&EpiloguePage>) {
        self.page = page.map(Encoded::new);
        item.dirty = true;
    }

    pub fn set_colors(&mut self, item: &mut ItemState, colors: [RgbColor; 3]) {
        self.heading_color = colors[2];
        item.dirty = true;
    }
}

impl ControlProc for Report {
    fn draw(&self, item: &ItemState, ctx: &DrawContext<'_>, bitmap: &mut Bitmap32) {
        let (Some(page), Some(heading_font), Some(table_font)) = (
            &self.page,
            ctx.res.font(HEADING_FONT),
            ctx.res.font(TABLE_FONT),
        ) else {
            return;
        };
        let mut strings = page.strings.iter().map(Vec::as_slice);
        let mut y = item.rect.top;
        for section in &page.sections {
            let rect = Rect::new(
                y,
                item.rect.left,
                y + section.rows * ROW_HEIGHT + HEADING_HEIGHT,
                item.rect.left + HEADING_WIDTH,
            );
            y = rect.bottom + SECTION_GAP;
            font::draw_string(
                bitmap,
                rect,
                TextClip::None,
                heading_font,
                font::STYLE_PLAIN,
                strings.next().unwrap_or_default(),
                self.heading_color,
                text_flags::SHADOW,
                0,
                ctx.res.locale(),
            );
            for row in 0..section.rows {
                let top = rect.top + HEADING_HEIGHT + row * ROW_HEIGHT;
                let color = if row == 0 {
                    self.header_color
                } else {
                    self.row_color
                };
                for (column, right) in section.columns.iter().enumerate() {
                    let flags = if column == 0 {
                        text_flags::SHADOW
                    } else {
                        text_flags::SHADOW | text_flags::RIGHT
                    };
                    font::draw_string(
                        bitmap,
                        Rect::new(top, rect.left, top + ROW_HEIGHT, rect.left + right),
                        TextClip::None,
                        table_font,
                        font::STYLE_PLAIN,
                        strings.next().unwrap_or_default(),
                        color,
                        flags,
                        0,
                        ctx.res.locale(),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_session::Text;

    fn text(s: &str) -> Text {
        Text(s.as_bytes().to_vec())
    }

    fn table(rows: usize) -> ReportSection {
        ReportSection {
            kind: SectionKind::Kills,
            heading: text("36 Kills"),
            columns: vec![text("Unit"), text("Dead")],
            rows: vec![vec![text("Ghasts"), text("10")]; rows],
        }
    }

    fn line(kind: SectionKind) -> ReportSection {
        ReportSection {
            kind,
            heading: text("Damage Ratio 1.78:1"),
            columns: Vec::new(),
            rows: Vec::new(),
        }
    }

    #[test]
    fn sections_walk_in_the_order_written_with_the_engines_counts() {
        let encoded = Encoded::new(&EpiloguePage {
            title: text("Salvation (Heroic)"),
            sections: vec![table(2), line(SectionKind::DamageRatio)],
        });
        let rows: Vec<i32> = encoded.sections.iter().map(|s| s.rows).collect();
        assert_eq!(rows, [0, 3, 0]);
        assert_eq!(encoded.sections[1].columns, [150, 200]);
        let strings: Vec<&[u8]> = encoded.strings.iter().map(Vec::as_slice).collect();
        assert_eq!(
            strings,
            [
                &b"Salvation (Heroic)"[..],
                b"36 Kills",
                b"Unit",
                b"Dead",
                b"Ghasts",
                b"10",
                b"Ghasts",
                b"10",
                b"Damage Ratio 1.78:1",
            ]
        );
    }

    #[test]
    fn a_string_past_the_limit_is_dropped_and_the_rest_move_up() {
        let mut encoder = Encoder::default();
        encoder.push(&[b'a'; 1000]);
        encoder.push(&[b'b'; 22]);
        assert_eq!(encoder.strings.len(), 1, "1001 + 23 is 0x400");
        encoder.push(&[b'c'; 21]);
        assert_eq!(encoder.strings.len(), 2);
        assert_eq!(encoder.strings[1], [b'c'; 21]);
        assert_eq!(encoder.used, 1023);
        encoder.push(b"");
        assert_eq!(
            encoder.strings.len(),
            2,
            "even an empty string needs a byte"
        );
    }
}
