use mandrake_common::{FourCC, M2Error};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Rect {
    pub top: i32,
    pub left: i32,
    pub bottom: i32,
    pub right: i32,
}

impl Rect {
    pub const fn new(top: i32, left: i32, bottom: i32, right: i32) -> Self {
        Self {
            top,
            left,
            bottom,
            right,
        }
    }

    pub const fn width(&self) -> i32 {
        self.right - self.left
    }

    pub const fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub const fn offset(&self, dx: i32, dy: i32) -> Self {
        Self::new(
            self.top + dy,
            self.left + dx,
            self.bottom + dy,
            self.right + dx,
        )
    }

    pub const fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

impl std::fmt::Display for Rect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "x {}..{} y {}..{} ({}x{})",
            self.left,
            self.right,
            self.top,
            self.bottom,
            self.width(),
            self.height()
        )
    }
}

pub const ITEM_STATIC_TEXT: FourCC = FourCC(*b"stat");
pub const ITEM_BUTTON: FourCC = FourCC(*b"butt");
pub const ITEM_MONSTER_STATUS: FourCC = FourCC(*b"STAT");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogItem {
    pub kind: FourCC,
    pub id: u32,
    pub field_04: i16,
    pub fields_08: [i16; 4],
    pub rect: Rect,
    pub name: Vec<u8>,
}

impl DialogItem {
    pub const fn parked(&self) -> bool {
        self.rect.left >= 0x4000 && self.rect.right >= 0x4000
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogDefinition {
    pub version: u16,
    pub rect: Rect,
    pub items: Vec<DialogItem>,
}

pub const DIALOG_HEADER_SIZE: usize = 12;
pub const DIALOG_ITEM_SIZE: usize = 24;

fn be16(d: &[u8], o: usize) -> i16 {
    i16::from_be_bytes([d[o], d[o + 1]])
}

fn rect_at(d: &[u8], o: usize) -> Rect {
    Rect::new(
        be16(d, o) as i32,
        be16(d, o + 2) as i32,
        be16(d, o + 4) as i32,
        be16(d, o + 6) as i32,
    )
}

impl DialogDefinition {
    pub fn from_bytes(d: &[u8]) -> Result<Self, M2Error> {
        if d.len() < DIALOG_HEADER_SIZE {
            return Err(M2Error::TagParse("bina: shorter than its header".into()));
        }
        let version = be16(d, 0) as u16;
        let count = be16(d, 2);
        if count < 0 {
            return Err(M2Error::TagParse(format!("bina: item count {count}")));
        }
        let count = count as usize;
        let tail = DIALOG_HEADER_SIZE + DIALOG_ITEM_SIZE * count;
        if d.len() < tail {
            return Err(M2Error::TagParse(format!(
                "bina: {count} items need {tail} bytes, have {}",
                d.len()
            )));
        }
        let mut names = d[tail..].split(|&b| b == 0);
        let items = (0..count)
            .map(|i| {
                let o = DIALOG_HEADER_SIZE + DIALOG_ITEM_SIZE * i;
                DialogItem {
                    kind: FourCC::new([d[o], d[o + 1], d[o + 2], d[o + 3]]),
                    field_04: be16(d, o + 4),
                    id: be16(d, o + 6) as u32,
                    fields_08: [
                        be16(d, o + 8),
                        be16(d, o + 10),
                        be16(d, o + 12),
                        be16(d, o + 14),
                    ],
                    rect: rect_at(d, o + 16),
                    name: names.next().map_or_else(Vec::new, <[u8]>::to_vec),
                }
            })
            .collect();
        Ok(Self {
            version,
            rect: rect_at(d, 4),
            items,
        })
    }

    pub fn item(&self, id: u32) -> Option<&DialogItem> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn apply_ditl(&mut self, ditl: &[u8]) {
        let lines = parse_ditl(ditl);
        for item in &mut self.items {
            item.name = (item.id as usize)
                .checked_sub(1)
                .and_then(|i| lines.get(i))
                .cloned()
                .unwrap_or_default();
        }
    }

    pub fn width(&self) -> i32 {
        self.rect.width()
    }

    pub fn height(&self) -> i32 {
        self.rect.height()
    }
}

fn expand_line_breaks(line: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(line.len());
    let mut at = 0;
    while at < line.len() {
        match (line[at], line.get(at + 1)) {
            (b'|', Some(b'r')) => out.extend_from_slice(b"\r\n"),
            (b'|', Some(&next)) => out.extend_from_slice(&[b'|', next]),
            (byte, _) => {
                out.push(byte);
                at += 1;
                continue;
            }
        }
        at += 2;
    }
    out
}

pub fn parse_ditl(d: &[u8]) -> Vec<Vec<u8>> {
    let mut v: Vec<Vec<u8>> = d.split(|&b| b == b'\r').map(expand_line_breaks).collect();
    if v.last().is_some_and(Vec::is_empty) {
        v.pop();
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_parses_items_and_names() {
        let mut d = vec![0, 1, 0, 1, 0x02, 0xe0, 0, 0, 3, 0, 4, 0];
        d.extend_from_slice(b"butt");
        d.extend_from_slice(&[0, 0, 0, 3, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0]);
        d.extend_from_slice(&[0, 1, 0, 2, 0, 3, 0, 4]);
        d.extend_from_slice(b"Stop\0");
        let dl = DialogDefinition::from_bytes(&d).unwrap();
        assert_eq!(dl.version, 1);
        assert_eq!(dl.rect, Rect::new(736, 0, 768, 1024));
        assert_eq!(dl.items[0].kind, ITEM_BUTTON);
        assert_eq!(dl.items[0].id, 3);
        assert_eq!(dl.items[0].field_04, 0);
        assert_eq!(dl.items[0].fields_08, [-1, -1, 0, 0]);
        assert_eq!(dl.items[0].rect, Rect::new(1, 2, 3, 4));
        assert_eq!(dl.items[0].name, b"Stop");
        assert!(DialogDefinition::from_bytes(&d[..20]).is_err());
    }

    #[test]
    fn ditl_replaces_names_by_id() {
        let mut d = vec![0, 1, 0, 2, 0, 0, 0, 0, 1, 0, 1, 0];
        for id in [2u8, 5] {
            d.extend_from_slice(b"butt");
            d.extend_from_slice(&[0, 0, 0, id, 0, 0, 0, 0, 0, 0, 0, 0]);
            d.extend_from_slice(&[0x40, 0, 0x40, 0, 0x40, 0, 0x40, 0]);
        }
        d.extend_from_slice(b"own\0own\0");
        let mut dl = DialogDefinition::from_bytes(&d).unwrap();
        assert!(dl.items[0].parked());
        dl.apply_ditl(b"one\rtwo\rthree\r");
        assert_eq!(dl.items[0].name, b"two");
        assert_eq!(dl.items[1].name, b"");
    }

    #[test]
    fn ditl_names() {
        assert_eq!(
            parse_ditl(b"Whisper\rAlly\rStop\r"),
            vec![b"Whisper".to_vec(), b"Ally".to_vec(), b"Stop".to_vec()]
        );
    }

    #[test]
    fn rect_helpers() {
        let r = Rect::new(7, 30, 21, 872);
        assert_eq!((r.width(), r.height()), (842, 14));
        assert_eq!(r.offset(1, 1), Rect::new(8, 31, 22, 873));
        assert!(r.contains(30, 7) && !r.contains(872, 7) && !r.contains(30, 21));
    }
}
