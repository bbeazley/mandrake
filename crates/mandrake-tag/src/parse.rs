use std::ops::Range;

use mandrake_common::FourCC;

pub(crate) fn be_i16(data: &[u8], offset: usize) -> Option<i16> {
    let end = offset.checked_add(2)?;
    let bytes: [u8; 2] = data.get(offset..end)?.try_into().ok()?;
    Some(i16::from_be_bytes(bytes))
}

pub(crate) fn be_u16(data: &[u8], offset: usize) -> Option<u16> {
    let end = offset.checked_add(2)?;
    let bytes: [u8; 2] = data.get(offset..end)?.try_into().ok()?;
    Some(u16::from_be_bytes(bytes))
}

pub(crate) fn be_i32(data: &[u8], offset: usize) -> Option<i32> {
    let end = offset.checked_add(4)?;
    let bytes: [u8; 4] = data.get(offset..end)?.try_into().ok()?;
    Some(i32::from_be_bytes(bytes))
}

pub(crate) fn be_u32(data: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let bytes: [u8; 4] = data.get(offset..end)?.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

pub(crate) fn nonneg_i32(data: &[u8], offset: usize) -> Option<usize> {
    usize::try_from(be_i32(data, offset)?).ok()
}

pub(crate) fn checked_table_range(
    base: usize,
    offset: usize,
    count: usize,
    stride: usize,
    len: usize,
) -> Option<Range<usize>> {
    let start = base.checked_add(offset)?;
    let bytes = count.checked_mul(stride)?;
    let end = start.checked_add(bytes)?;
    (end <= len).then_some(start..end)
}

pub(crate) fn optional_fourcc(data: &[u8], offset: usize) -> Option<FourCC> {
    let end = offset.checked_add(4)?;
    let bytes: [u8; 4] = data.get(offset..end)?.try_into().ok()?;
    (bytes != [0; 4] && bytes != [0xff; 4]).then_some(FourCC::new(bytes))
}

pub(crate) fn until_nul(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len())]
}

pub(crate) fn is_plugin_url_description(description: &[u8]) -> bool {
    description.is_empty()
        || [&b"http:"[..], b"www.", b".com", b".net"]
            .iter()
            .any(|needle| description.windows(needle.len()).any(|w| w == *needle))
}
