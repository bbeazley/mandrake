use mandrake_common::M2Error;

use crate::header::{SBEntryTag, SBMonoHeader, SBTagHeader};

#[derive(Debug, Clone)]
pub struct MonolithTag {
    pub header: SBTagHeader,
    pub data: Vec<u8>,
}

pub fn write_monolith(
    header: &SBMonoHeader,
    entries: &[SBEntryTag],
    tags: &[MonolithTag],
) -> Result<Vec<u8>, M2Error> {
    let count = |n: usize, what: &str| {
        u16::try_from(n).map_err(|_| M2Error::TagParse(format!("too many {what}: {n}")))
    };
    let directory = SBMonoHeader::SIZE + entries.len() * SBEntryTag::SIZE;
    let data_start = directory + tags.len() * SBTagHeader::SIZE;
    let total = data_start + tags.iter().map(|t| t.data.len()).sum::<usize>();
    let total = u32::try_from(total)
        .ok()
        .filter(|&t| t <= i32::MAX as u32)
        .ok_or_else(|| M2Error::TagParse(format!("archive too large: {total} bytes")))?;

    let mut out = Vec::with_capacity(total as usize);
    SBMonoHeader {
        entry_tag_count: count(entries.len(), "entry tags")?,
        tag_list_count: count(tags.len(), "tags")?,
        size: total,
        ..header.clone()
    }
    .write(&mut out)?;
    for entry in entries {
        entry.write(&mut out)?;
    }
    let mut offset = data_start;
    for tag in tags {
        SBTagHeader {
            tag_data_offset: offset as i32,
            tag_data_size: tag.data.len() as i32,
            signature: SBTagHeader::SIGNATURE,
            ..tag.header.clone()
        }
        .write(&mut out)?;
        offset += tag.data.len();
    }
    for tag in tags {
        out.extend_from_slice(&tag.data);
    }
    debug_assert_eq!(out.len(), total as usize);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TagArchive;
    use crate::header::encode_c_string;
    use mandrake_common::FourCC;

    fn tag(group: &[u8; 4], id: &[u8; 4], name: &str, data: &[u8]) -> MonolithTag {
        MonolithTag {
            header: SBTagHeader {
                identifier: 7,
                flags: 0,
                tag_type_idx: 1,
                name: encode_c_string(name),
                tag_type: FourCC::new(*group),
                tag_id: FourCC::new(*id),
                tag_data_offset: -1,
                tag_data_size: -1,
                user_data: 0x1234_5678,
                version: 5,
                destination: -1,
                owner_index: -1,
                signature: FourCC::new(*b"xxxx"),
            },
            data: data.to_vec(),
        }
    }

    #[test]
    fn written_archive_reads_back_with_the_same_tags() {
        let header = SBMonoHeader {
            r#type: 1,
            version: 0,
            name: encode_c_string("Test Plugin"),
            description: encode_c_string("written by mandrake-tag"),
            entry_tag_count: 99,
            tag_list_count: 99,
            checksum: [1, 2, 3, 4],
            flags: 1,
            size: 0,
            header_checksum: [5, 6, 7, 8],
            padding: [0; 4],
            signature: SBMonoHeader::SIG_DNG2,
        };
        let entry = SBEntryTag {
            tag_id: FourCC::new(*b"85gs"),
            reserved1: [0; 12],
            internal_name: encode_c_string("61 gimble sandy"),
            display_name: encode_c_string("Gimble in the Wabe (Sandy)"),
            description: [0; 32],
        };
        let tags = vec![
            tag(b"mesh", b"85gs", "61 gimble sandy", b"mesh payload"),
            tag(b".256", b"85gs", "85 gimble sandy", &[0xab; 1000]),
            tag(b"prel", b"85gs", "empty", b""),
        ];
        let bytes = write_monolith(&header, std::slice::from_ref(&entry), &tags).unwrap();
        let len = bytes.len();
        let archive = TagArchive::from_bytes(bytes).unwrap();

        let h = archive.header();
        assert_eq!(h.entry_tag_count, 1);
        assert_eq!(h.tag_list_count, 3);
        assert_eq!(h.size as usize, len);
        assert_eq!(h.name_as_str(), "Test Plugin");
        assert_eq!(h.checksum, [1, 2, 3, 4]);
        assert_eq!(h.signature, SBMonoHeader::SIG_DNG2);
        assert_eq!(archive.entry_tags(), [entry]);

        assert_eq!(archive.tag_count(), tags.len());
        for (want, got) in tags.iter().zip(archive.list_tags()) {
            assert_eq!(got.tag_type, want.header.tag_type);
            assert_eq!(got.tag_id, want.header.tag_id);
            assert_eq!(got.name, want.header.name_as_str());
            assert_eq!(got.header.user_data, want.header.user_data);
            assert_eq!(got.header.version, want.header.version);
            assert_eq!(got.header.identifier, want.header.identifier);
            assert_eq!(got.header.signature, SBTagHeader::SIGNATURE);
            assert_eq!(archive.get_tag_data(got).unwrap(), want.data);
        }
    }
}
