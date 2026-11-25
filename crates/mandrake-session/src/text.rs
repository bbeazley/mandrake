//! Text as the tags hold it.

use mandrake_common::mac_roman;

/// Bytes as the tags hold them: Mac Roman, with the font's `|` style codes (`mandrake_tag::font`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Text(pub Vec<u8>);

impl Text {
    /// UTF-8 with the style codes taken out, for any front end and for the stdout printer. As the
    /// font reads them: `|X` is a two-byte code unless X is a carriage return, and `||` is `|`. `|x`
    /// hides what follows, which the font spaces for and doesn't draw (`TextCursor`), so it isn't here
    /// at all, up to what sets a colour again: `|p`, `|l`, `|0`..`|9` or `|A`..`|Z`.
    pub fn to_plain_utf8(&self) -> String {
        if !self.0.contains(&b'|') {
            return mac_roman::decode(&self.0);
        }
        let mut visible = Vec::with_capacity(self.0.len());
        let mut bytes = self.0.iter().copied().peekable();
        let mut hidden = false;
        while let Some(b) = bytes.next() {
            if b == b'|' {
                match bytes.peek() {
                    Some(b'\r') | None => {}
                    Some(&code) => {
                        bytes.next();
                        match code {
                            b'|' if !hidden => visible.push(b'|'),
                            b'x' => hidden = true,
                            b'p' | b'l' | b'0'..=b'9' | b'A'..=b'Z' => hidden = false,
                            _ => {}
                        }
                        continue;
                    }
                }
            }
            if !hidden {
                visible.push(b);
            }
        }
        mac_roman::decode(&visible)
    }

    /// The other way, for text that is already plain: Mac Roman, with `|` as `||` so that it isn't
    /// read as a code. A character Mac Roman doesn't have becomes `?`.
    pub fn from_plain_utf8(plain: &str) -> Self {
        let encoded = mac_roman::encode(plain);
        if !encoded.contains(&b'|') {
            return Self(encoded);
        }
        let mut bytes = Vec::with_capacity(encoded.len() + 4);
        for b in encoded {
            if b == b'|' {
                bytes.push(b'|');
            }
            bytes.push(b);
        }
        Self(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_decodes_mac_roman() {
        assert_eq!(Text(vec![b'a', 0x8e, 0xd2, 0xd3]).to_plain_utf8(), "aé“”");
        let all: Vec<u8> = (0..=255).filter(|&b| b != b'|').collect();
        assert_eq!(
            Text::from_plain_utf8(&Text(all.clone()).to_plain_utf8()).0,
            all
        );
    }

    #[test]
    fn plain_text_drops_style_codes_as_the_font_reads_them() {
        let plain = |s: &[u8]| Text(s.to_vec()).to_plain_utf8();
        assert_eq!(plain(b"|bBold|p plain"), "Bold plain");
        assert_eq!(plain(b"a||b"), "a|b");
        assert_eq!(plain(b"a|\rb"), "a|\rb");
        assert_eq!(plain(b"end|"), "end|");
    }

    #[test]
    fn plain_text_leaves_out_hidden_text_as_the_font_draws_it() {
        let plain = |s: &[u8]| Text(s.to_vec()).to_plain_utf8();
        assert_eq!(plain(b"a|xhidden|p b"), "a b");
        assert_eq!(plain(b"a|xhidden|3b"), "ab");
        assert_eq!(plain(b"a|xhidden|Zb"), "ab");
        assert_eq!(plain(b"a|xhidden|lb"), "ab");
        assert_eq!(plain(b"a|xhidden||more|bstill"), "a");
        assert_eq!(plain(b"a|xto the end"), "a");
    }

    #[test]
    fn plain_text_goes_back_to_the_same_bytes() {
        for text in ["Ghôls", "a|b", "“é”", "plain"] {
            assert_eq!(Text::from_plain_utf8(text).to_plain_utf8(), text);
        }
        assert_eq!(Text::from_plain_utf8("é").0, [0x8e]);
        assert_eq!(Text::from_plain_utf8("a|b").0, b"a||b");
        assert_eq!(Text::from_plain_utf8("\u{3b1}").0, b"?");
    }
}
