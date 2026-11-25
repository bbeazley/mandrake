//! Prints a dialog's items as the menus' tags have them: `items ID [--stock]`. With `STRINGS=1` it
//! prints the string list `ID` instead, and with `ART=1` the sizes of `.256 ID`'s first bitmaps.

use mandrake_common::FourCC;
use mandrake_dialogs::resources::Resources;
use mandrake_tag::TagLookup;
use mandrake_tag::tag_files::{INTERFACE_800X600, dialog_tag_files};

fn main() {
    let mut args = std::env::args().skip(1);
    let id = args.next().expect("usage: items ID [--stock]");
    let stock = args.next().as_deref() == Some("--stock");
    let game =
        std::path::PathBuf::from(std::env::var_os("MYTH2_DATA_DIR").expect("MYTH2_DATA_DIR"));
    let files = dialog_tag_files(&game, (!stock).then_some(INTERFACE_800X600));
    let mut res = Resources::new(TagLookup::open(&files));
    let mut four = [b' '; 4];
    four[..id.len().min(4)].copy_from_slice(&id.as_bytes()[..id.len().min(4)]);
    if std::env::var_os("ART").is_some() {
        let art = res.art(FourCC(four));
        println!("art {id}: {}", art.is_some());
        if let Some(art) = art {
            for seq in 0..6 {
                println!(
                    "  seq {seq}: {:?}",
                    art.bitmap(seq, -1, 0).map(|(b, _)| (b.width, b.height))
                );
            }
        }
        return;
    }
    if let Some(data) = res
        .tag(FourCC(*b"stli"), FourCC(four))
        .filter(|_| std::env::var_os("STRINGS").is_some())
    {
        for (i, line) in mandrake_tag::mesh_links::string_list_lines(&data)
            .iter()
            .enumerate()
        {
            println!("{i:>3} {:?}", String::from_utf8_lossy(line));
        }
        return;
    }
    let def = res.dialog(FourCC(four)).expect("no such dialog");
    println!("{id}: {:?} {}x{}", def.rect, def.width(), def.height());
    for item in &def.items {
        println!(
            "{:>4} {} {:?} {:?} {:?}",
            item.id,
            item.kind,
            item.fields_08,
            item.rect,
            String::from_utf8_lossy(&item.name)
        );
    }
}
