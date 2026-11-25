//! Mac Roman, the engine's text: a film's and a tag's names are their bytes. Every byte is a
//! character, so a name decoded and encoded again is the same bytes.

/// Mac Roman's upper half, `0x80..=0xff`.
const UPPER: [char; 128] = [
    'Ä', 'Å', 'Ç', 'É', 'Ñ', 'Ö', 'Ü', 'á', 'à', 'â', 'ä', 'ã', 'å', 'ç', 'é', 'è', //
    'ê', 'ë', 'í', 'ì', 'î', 'ï', 'ñ', 'ó', 'ò', 'ô', 'ö', 'õ', 'ú', 'ù', 'û', 'ü', //
    '†', '°', '¢', '£', '§', '•', '¶', 'ß', '®', '©', '™', '´', '¨', '≠', 'Æ', 'Ø', //
    '∞', '±', '≤', '≥', '¥', 'µ', '∂', '∑', '∏', 'π', '∫', 'ª', 'º', 'Ω', 'æ', 'ø', //
    '¿', '¡', '¬', '√', 'ƒ', '≈', '∆', '«', '»', '…', '\u{a0}', 'À', 'Ã', 'Õ', 'Œ', 'œ', //
    '–', '—', '“', '”', '‘', '’', '÷', '◊', 'ÿ', 'Ÿ', '⁄', '€', '‹', '›', 'ﬁ', 'ﬂ', //
    '‡', '·', '‚', '„', '‰', 'Â', 'Ê', 'Á', 'Ë', 'È', 'Í', 'Î', 'Ï', 'Ì', 'Ó', 'Ô', //
    '\u{f8ff}', 'Ò', 'Ú', 'Û', 'Ù', 'ı', 'ˆ', '˜', '¯', '˘', '˙', '˚', '¸', '˝', '˛', 'ˇ', //
];

/// The text of Mac Roman bytes.
pub fn decode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0..=0x7f => char::from(b),
            _ => UPPER[usize::from(b - 0x80)],
        })
        .collect()
}

/// The text of a NUL-ended name: the bytes up to the first NUL, or all of them.
pub fn decode_c(bytes: &[u8]) -> String {
    decode(&bytes[..bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len())])
}

/// `bytes` without the ASCII whitespace at either end (what `str::trim` takes off a name whose
/// bytes are ASCII). Mac Roman's `0xca`, a no-break space, stays: it is a byte of the name.
pub fn trim(bytes: &[u8]) -> &[u8] {
    let space = |b: &u8| b.is_ascii() && char::from(*b).is_whitespace();
    let start = bytes.iter().position(|b| !space(b)).unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !space(b))
        .map_or(start, |e| e + 1);
    &bytes[start..end]
}

/// The Mac Roman bytes of a text; a character Mac Roman doesn't have becomes `?`.
pub fn encode(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| match u8::try_from(u32::from(c)) {
            Ok(b) if b < 0x80 => b,
            _ => UPPER
                .iter()
                .position(|&m| m == c)
                .map_or(b'?', |i| 0x80 + i as u8),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_comes_back() {
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(encode(&decode(&all)), all);
        assert_eq!(encode("日"), b"?");
    }

    #[test]
    fn names_end_and_trim_on_bytes() {
        assert_eq!(decode_c(b"Gh\xbfls\0junk"), "Ghøls");
        assert_eq!(trim(b" \tA\xca \x0b"), b"A\xca");
        assert_eq!(trim(b"   "), b"");
    }

    /// Two player names of the network films that aren't ASCII.
    #[test]
    fn non_ascii_player_names() {
        assert_eq!(decode(b"NAZI\xaa"), "NAZI™");
        assert_eq!(decode(b"Hairpin Gh\xbfls"), "Hairpin Ghøls");
        assert_eq!(encode("Hairpin Ghøls"), b"Hairpin Gh\xbfls");
    }
}
