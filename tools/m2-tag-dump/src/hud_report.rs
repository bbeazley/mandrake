use mandrake_common::FourCC;
use mandrake_tag::hud::{
    CONTROL_BAR_ENTRIES, STATUS_BAR_ITEM_FLAGS, control_bar_layout, control_bar_level,
    control_bar_row_width, minimap_base_scale, minimap_rect, status_bar_item_rect,
};
use mandrake_tag::{CollectionTag, DialogDefinition, FontTag, MeshTag, StringListTag};

type Tags<'a> = &'a dyn Fn([u8; 4], FourCC) -> Option<Vec<u8>>;

fn be32(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4)
        .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn tag_ref(d: &[u8], o: usize) -> Option<FourCC> {
    let b = d.get(o..o + 4)?;
    (b != [0xff; 4] && b != [0; 4]).then(|| FourCC::new([b[0], b[1], b[2], b[3]]))
}

fn print_collection(tags: Tags, id: FourCC, what: &str) -> Option<CollectionTag> {
    let Some(raw) = tags(*b".256", id) else {
        println!("  .256 {id} ({what}): missing");
        return None;
    };
    let coll = CollectionTag::from_bytes(&raw).ok()?;
    println!(
        "  .256 {id} ({what}): {} bitmaps, {} sequences",
        coll.frame_count(),
        coll.sequences.len()
    );
    for s in &coll.sequences {
        let bitmaps: Vec<String> = (0..s.frames_per_view as usize)
            .map(|f| {
                (0..s.num_views.max(1) as usize)
                    .map(|v| match s.bitmap_index(v, f).and_then(|b| coll.frame(b)) {
                        Some(b) => format!("#{} {}x{}", b.index, b.width, b.height),
                        None => "-".into(),
                    })
                    .collect::<Vec<_>>()
                    .join(" / ")
            })
            .collect();
        println!(
            "    sequence {:2} '{}': {} views x {} frames: {}",
            s.index,
            s.name,
            s.num_views,
            s.frames_per_view,
            bitmaps.join(", ")
        );
    }
    Some(coll)
}

fn print_strings(tags: Tags, id: FourCC, what: &str) {
    match tags(*b"stli", id).and_then(|d| StringListTag::from_bytes(&d).ok()) {
        Some(s) => {
            let v: Vec<String> = s.iter().map(|x| format!("{x:?}")).collect();
            println!("  stli {id} ({what}): {}", v.join(", "));
        }
        None => println!("  stli {id} ({what}): missing"),
    }
}

fn sequence_size(coll: &CollectionTag, seq: usize) -> (i32, i32) {
    coll.sequence(seq)
        .and_then(|s| s.bitmap_index(0, 0))
        .and_then(|b| coll.frame(b))
        .map_or((0, 0), |b| (b.width as i32, b.height as i32))
}

pub fn hud_report(tags: Tags, meshes: &[FourCC], sizes: &[(i32, i32)]) -> anyhow::Result<()> {
    let inte_id = FourCC::new(*b"stin");
    println!("# inte {inte_id} (as the engine sets it up for a new map)");
    if let Some(d) = tags(*b"inte", inte_id) {
        println!(
            "  +0x00 collection {} -> [0x860870]+0xfc (get_interface_collection)",
            tag_ref(&d, 0).map_or("-".into(), |c| c.to_string())
        );
        for i in 0..6 {
            let f = tag_ref(&d, 4 + 4 * i);
            let name = f
                .and_then(|f| tags(*b"font", f).map(|d| (f, FontTag::from_bytes(&d))))
                .map_or("missing".to_string(), |(f, t)| match t {
                    Ok(t) => format!(
                        "font {f}: asc {} desc {} leading {} margin {}, {} glyphs, styles [{}]",
                        t.ascending,
                        t.descending,
                        t.leading,
                        t.margin(),
                        t.glyphs.len(),
                        t.style_fonts
                            .iter()
                            .map(|s| s.map_or("-".into(), |s| s.to_string()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    Err(e) => format!("font {f}: {e}"),
                });
            println!(
                "  +{:#04x} interface font {i} {} -> [0x860870]+{:#x} (get_interface_font_index): {name}",
                4 + 4 * i,
                f.map_or("-".into(), |c| c.to_string()),
                0xec + 2 * i
            );
        }
        if let Some(c) = tag_ref(&d, 0) {
            print_collection(tags, c, "interface collection");
        }
    } else {
        println!("  missing");
    }
    println!();

    let sba = FourCC::new(*b"!sba");
    let cba = FourCC::new(*b"!cba");
    println!("# status bar (initialize_status_bar_for_new_map, update_status_bar)");
    let status = tags(*b"bina", sba).and_then(|d| DialogDefinition::from_bytes(&d).ok());
    if let Some(dl) = &status {
        println!("  bina {sba}: dialog {}; {} items", dl.rect, dl.items.len());
        for it in &dl.items {
            let shift = STATUS_BAR_ITEM_FLAGS
                .iter()
                .find(|e| e.0 == it.id)
                .map_or("fixed".to_string(), |e| format!("table flags {}", e.1));
            print!("    item {:2} {} {} ({shift})", it.id, it.kind, it.rect);
            for &(w, _) in sizes {
                print!("; at {w}: {}", status_bar_item_rect(it.id, it.rect, w));
            }
            println!();
        }
    }
    if let (Some(coll), Some(raw)) = (
        print_collection(tags, sba, "status bar art, [0x9895d8]"),
        tags(*b".256", sba),
    ) {
        let c: Vec<String> = (1..=3)
            .map(|e| {
                let c = mandrake_tag::hud::rgb_color_from_sequence(&raw, &coll, 9, e);
                format!("{e} {:04x}/{:04x}/{:04x}", c[0], c[1], c[2])
            })
            .collect();
        println!(
            "    text colours (get_rgb_color_from_sequence, sequence 9; 1 description, 2 title, 3 rules): {}",
            c.join(", ")
        );
    }
    print_collection(tags, FourCC::new(*b"isb6"), "experience icons, [0x9895dc]");
    print_strings(
        tags,
        FourCC::new(*b"indl"),
        "difficulty names, title '%s  (%s)' 0x618230",
    );
    print_strings(tags, FourCC::new(*b"inom"), "overhead map popup");
    println!();

    println!("# control bar (as the engine lays it out for a new map)");
    let control = tags(*b"bina", cba).and_then(|d| DialogDefinition::from_bytes(&d).ok());
    let captions: Vec<String> = control.as_ref().map_or_else(Vec::new, |dl| {
        dl.items
            .iter()
            .map(|i| String::from_utf8_lossy(&i.name).into_owned())
            .collect()
    });
    if let Some(dl) = &control {
        println!(
            "  bina {cba}: dialog {}; {} items; captions {:?}",
            dl.rect,
            dl.items.len(),
            captions
        );
    }
    print_strings(
        tags,
        FourCC::new(*b"incb"),
        "control bar strings, [0x86b68a]",
    );
    if let (Some(coll), Some(dl)) = (
        print_collection(tags, cba, "control bar art, [0x86b688]"),
        &control,
    ) {
        let bar_h = dl.height();
        let sizes_b: Vec<(i32, i32)> = CONTROL_BAR_ENTRIES
            .iter()
            .map(|e| sequence_size(&coll, e.sequence as usize))
            .collect();
        let widths: Vec<i32> = sizes_b.iter().map(|s| s.0).collect();
        for &(w, h) in sizes {
            let level = control_bar_level(&CONTROL_BAR_ENTRIES, &widths, w);
            println!(
                "  at {w}x{h}: bar y {}..{h}, level {level}, row width {}:",
                h - bar_h,
                control_bar_row_width(&CONTROL_BAR_ENTRIES, &widths, level)
            );
            for (item, r) in control_bar_layout(&CONTROL_BAR_ENTRIES, &sizes_b, w, bar_h) {
                let name = captions.get(item as usize - 1).cloned().unwrap_or_default();
                let seq = CONTROL_BAR_ENTRIES[item as usize - 1].sequence;
                let seqname = coll
                    .sequence(seq as usize)
                    .map_or(String::new(), |s| s.name.clone());
                println!(
                    "    item {item:2} {name:8} sequence {seq:2} '{seqname}': x {}..{} y {}..{}",
                    r.left,
                    r.right,
                    h - bar_h + r.top,
                    h - bar_h + r.bottom
                );
            }
        }
    }
    println!();

    let status_h = status.as_ref().map_or(64, DialogDefinition::height);
    for &id in meshes {
        let Some(md) = tags(*b"mesh", id) else {
            println!("# mesh {id}: missing");
            continue;
        };
        let mesh = MeshTag::from_bytes(&md)?;
        let flags = be32(&md, 0x4c);
        println!(
            "# mesh {id}: overhead map collection +0x98 {} (-> +0x28c), mesh flags +0x4c {flags:#010x} (0x80000: clamped map, initialize_overhead_map_for_new_game)",
            mesh.overhead_tag.map_or("-".into(), |c| c.to_string())
        );
        let Some(c) = mesh.overhead_tag else { continue };
        let Some(raw) = tags(*b".256", c) else {
            println!("  .256 {c}: missing");
            continue;
        };
        let coll = CollectionTag::from_bytes(&raw)?;
        let Some(b) = coll
            .sequence(0)
            .and_then(|s| s.bitmap_index(0, 0))
            .and_then(|i| coll.frame(i))
        else {
            println!("  .256 {c}: no sequence 0 bitmap");
            continue;
        };
        let (cw, ch) = mesh.dimensions();
        println!(
            "  .256 {c}: sequence 0 bitmap #{} {}x{} for {}x{} cells ({:.3} map pixels per cell)",
            b.index,
            b.width,
            b.height,
            cw,
            ch,
            b.width as f32 / cw as f32
        );
        for &(w, h) in sizes {
            let s = minimap_base_scale(b.width as i32, b.height as i32, w, h);
            println!(
                "    at {w}x{h}: base scale {s:.6}, minimap {}",
                minimap_rect(b.width as i32, b.height as i32, w, h, status_h)
            );
        }
    }
    Ok(())
}

pub const MESH_FLAG_EXPLORED_MAP: u32 = 0x0008_0000;
pub const MESH_FLAG_NO_MAP_AT_START: u32 = 0x0040_0000;
pub const MESH_FLAG_NO_HINTS_DIALOG: u32 = 0x0080_0000;

pub fn mesh_flags_report(archives: &[(String, mandrake_tag::TagArchive)]) -> anyhow::Result<()> {
    let mesh = FourCC::new(*b"mesh");
    println!(
        "# mesh flags +0x4c: {MESH_FLAG_EXPLORED_MAP:#x} explored-only overhead map, centred \
         ([0x970444]), {MESH_FLAG_NO_MAP_AT_START:#x} map hidden at start \
         (update_status_bar), {MESH_FLAG_NO_HINTS_DIALOG:#x} no hints dialog \
         (open_game_hints_dialog)"
    );
    let mut hidden = Vec::new();
    let mut total = 0;
    for (name, arc) in archives {
        for e in arc.find_tags_by_type(mesh) {
            let Ok(d) = arc.get_tag_data(e) else {
                println!("{name}\t{}\t{:?}\tunreadable", e.tag_id, e.name);
                continue;
            };
            let flags = be32(&d, 0x4c);
            let bit = |b: u32, s: &'static str| if flags & b != 0 { s } else { "-" };
            total += 1;
            println!(
                "{name}\t{}\t{:?}\t{flags:#010x}\t{}\t{}\t{}",
                e.tag_id,
                e.name,
                bit(MESH_FLAG_EXPLORED_MAP, "explored"),
                bit(MESH_FLAG_NO_MAP_AT_START, "map-hidden"),
                bit(MESH_FLAG_NO_HINTS_DIALOG, "no-hints"),
            );
            if flags & MESH_FLAG_NO_MAP_AT_START != 0 {
                hidden.push(format!("{} {:?} ({name})", e.tag_id, e.name));
            }
        }
    }
    println!(
        "# {total} mesh tags; {} with {MESH_FLAG_NO_MAP_AT_START:#x} (map hidden at start):",
        hidden.len()
    );
    for h in hidden {
        println!("#   {h}");
    }
    Ok(())
}
