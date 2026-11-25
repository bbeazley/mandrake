use std::io::Cursor;

use mandrake_common::FourCC;
use mandrake_tag::header::{SBEntryTag, SBMonoHeader, SBTagHeader};
use mandrake_tag::{TagArchive, tag_files};

fn plugin(checksum: u32, tags: &[(&[u8; 4], &[u8; 4], &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut name = [0u8; 32];
    name[..4].copy_from_slice(b"Fuzz");
    SBMonoHeader {
        r#type: tag_files::TYPE_PLUGIN,
        version: 1,
        name,
        description: [0; 64],
        entry_tag_count: 1,
        tag_list_count: tags.len() as u16,
        checksum: checksum.to_be_bytes(),
        flags: 0,
        size: 0,
        header_checksum: [0; 4],
        padding: [0; 4],
        signature: SBMonoHeader::SIG_MTH2,
    }
    .write(&mut out)
    .unwrap();
    SBEntryTag {
        tag_id: FourCC::new(*b"01xx"),
        reserved1: [0; 12],
        internal_name: [0; 32],
        display_name: [0; 32],
        description: [0; 32],
    }
    .write(&mut out)
    .unwrap();
    let mut offset = out.len() + tags.len() * SBTagHeader::SIZE;
    for (group, id, data) in tags {
        SBTagHeader {
            identifier: 0,
            flags: 0,
            tag_type_idx: 0,
            name: [0; 32],
            tag_type: FourCC::new(**group),
            tag_id: FourCC::new(**id),
            tag_data_offset: offset as i32,
            tag_data_size: data.len() as i32,
            user_data: 0,
            version: 0,
            destination: 0,
            owner_index: 0,
            signature: SBTagHeader::SIGNATURE,
        }
        .write(&mut out)
        .unwrap();
        offset += data.len();
    }
    for (_, _, data) in tags {
        out.extend_from_slice(data);
    }
    out
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn mutate(base: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut b = base.to_vec();
    let directory = SBMonoHeader::SIZE + SBEntryTag::SIZE + 3 * SBTagHeader::SIZE;
    for _ in 0..1 + rng.below(4) {
        let parsed = directory.min(b.len());
        if parsed < 4 {
            b.extend((0..rng.below(64)).map(|_| rng.next() as u8));
            continue;
        }
        match rng.below(6) {
            0 => {
                let i = rng.below(parsed);
                b[i] ^= 1 << rng.below(8);
            }
            1 => {
                let i = rng.below(parsed);
                b[i] = rng.next() as u8;
            }
            2 | 3 => {
                let width = if rng.below(2) == 0 { 2 } else { 4 };
                let i = rng.below(parsed.saturating_sub(width)) & !1;
                let v: [u8; 4] = match rng.below(4) {
                    0 => [0xff; 4],
                    1 => [0x7f, 0xff, 0xff, 0xff],
                    2 => [0x80, 0, 0, 0],
                    _ => (rng.next() as u32).to_be_bytes(),
                };
                b[i..i + width].copy_from_slice(&v[..width]);
            }
            4 => b.truncate(rng.below(b.len() + 1)),
            _ => b.extend((0..rng.below(64)).map(|_| rng.next() as u8)),
        }
    }
    b
}

fn read_all(bytes: &[u8]) -> usize {
    let mut read = 0;
    let _ = SBMonoHeader::read(&mut &bytes[..]);
    if let Ok(a) = TagArchive::from_bytes(bytes.to_vec()) {
        for e in a.list_tags() {
            read += a.get_tag_data(e).map_or(0, |d| d.len());
        }
    }
    if let Ok(a) = TagArchive::from_reader(Box::new(Cursor::new(bytes.to_vec()))) {
        for e in a.list_tags() {
            read += a.get_tag_data(e).map_or(0, |d| d.len());
            let _ = a.read_tag_prefix(e, 16);
        }
        let _ = a.find_tag_by_id(FourCC::new(*b"mesh"), FourCC::new(*b"aaaa"));
    }
    read
}

#[test]
fn malformed_plugins_are_refused_without_a_panic() {
    let base = plugin(
        0x1234_5678,
        &[
            (b"mesh", b"aaaa", b"the mesh's bytes"),
            (b"stli", b"bbbb", b"a string list"),
            (b".256", b"cccc", &[7; 40]),
        ],
    );
    assert_eq!(read_all(&base) / 2, 16 + 13 + 40);
    let mut rng = Rng(0x6d79_7468_3220_7334);
    let mut refused = 0;
    for _ in 0..20_000 {
        let m = mutate(&base, &mut rng);
        if TagArchive::from_bytes(m.clone()).is_err() {
            refused += 1;
        }
        read_all(&m);
    }
    assert!(refused > 1000, "most mutations break the file: {refused}");
}

#[test]
fn a_tag_past_the_end_is_an_error_from_a_stream() {
    let mut b = plugin(1, &[(b"mesh", b"aaaa", b"x")]);
    let size_at = SBMonoHeader::SIZE + SBEntryTag::SIZE + 0x30;
    b[size_at..size_at + 4].copy_from_slice(&0x7fff_ffffu32.to_be_bytes());
    let a = TagArchive::from_reader(Box::new(Cursor::new(b.clone()))).unwrap();
    assert!(a.get_tag_data(&a.list_tags()[0]).is_err());
    let a = TagArchive::from_bytes(b).unwrap();
    assert!(a.get_tag_data(&a.list_tags()[0]).is_err());
}

#[test]
fn the_downloads_folder_is_searched_after_the_games_plugins() {
    let base = std::env::temp_dir().join(format!("mandrake-tag-downloads-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (game, downloads) = (base.join("game"), base.join("downloads"));
    std::fs::create_dir_all(game.join("plugins")).unwrap();
    std::fs::create_dir_all(game.join("tags")).unwrap();
    std::fs::create_dir_all(downloads.join("partial")).unwrap();
    let file = plugin(0xfeed, &[(b"mesh", b"aaaa", b"downloaded")]);
    std::fs::write(downloads.join("Got It"), &file).unwrap();
    std::fs::write(downloads.join("Got It.part"), &file).unwrap();
    std::fs::write(downloads.join("partial").join("0000feed.part"), &file).unwrap();
    std::fs::write(downloads.join("Not A Plugin"), b"hello").unwrap();
    let names = |f: &tag_files::EngineTagFiles| -> Vec<String> {
        f.paths
            .iter()
            .map(|p| p.strip_prefix(&base).unwrap().display().to_string())
            .collect()
    };
    tag_files::set_downloads_dir(None);
    let f = tag_files::engine_tag_files(&game, &["Got It"]);
    assert_eq!(f.missing_plugins, ["Got It"]);
    tag_files::set_downloads_dir(Some(downloads.clone()));
    let f = tag_files::engine_tag_files(&game, &["got it", "Not A Plugin"]);
    assert_eq!(names(&f), ["downloads/Got It"]);
    assert_eq!(f.missing_plugins, ["Not A Plugin"]);
    std::fs::write(game.join("plugins").join("Got It"), &file).unwrap();
    let f = tag_files::engine_tag_files(&game, &["Got It"]);
    assert_eq!(names(&f), ["game/plugins/Got It"]);
    std::fs::remove_file(game.join("plugins").join("Got It")).unwrap();
    let mut rng = Rng(7);
    for i in 0..200 {
        std::fs::write(
            downloads.join(format!("m{}", i % 8)),
            mutate(&file, &mut rng),
        )
        .unwrap();
        let f = tag_files::engine_tag_files(&game, &["Got It"]);
        assert!(f.missing_plugins.is_empty());
        read_all(&std::fs::read(downloads.join(format!("m{}", i % 8))).unwrap());
    }
    tag_files::set_downloads_dir(None);
    std::fs::remove_dir_all(&base).unwrap();
}
