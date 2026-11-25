use crate::sprite_manager::{MAGENTA_ENTRY_16, keeps_color_alpha, sprite_texels};

fn bitmap(flags: u16, pixels: Vec<u8>) -> mandrake_tag::BitmapFrame {
    mandrake_tag::BitmapFrame {
        index: 0,
        name: String::new(),
        width: pixels.len() as u16,
        height: 1,
        offset_x: 0,
        offset_y: 0,
        flags,
        pixel_data_offset: 0,
        pixel_data_size: 0,
        pixels,
        wide_pixels: Vec::new(),
    }
}

#[test]
fn a_16_bit_bitmap_keeps_its_per_pixel_opacity() {
    let palette = [[0, 0, 255, 0], [100, 50, 200, 255]];
    let mut f = bitmap(0x13, vec![1, 1, 0, 1]);
    f.wide_pixels = vec![
        0x0001,
        0xf001,
        0x0800,
        mandrake_tag::collection::WIDE_PIXEL_OUTSIDE_RUN,
    ];
    let t = sprite_texels(&f, &palette, &[]);
    assert_eq!(t[0..4], [100, 50, 200, 255]);
    assert_eq!(t[4..8], [100, 50, 200, 255], "only the low nibble counts");
    assert_eq!(t[8..12], [0, 0, 0, 0x77]);
    assert_eq!(t[12..16], [0, 0, 0, 0], "outside a run");
}

#[test]
fn index_1_of_a_raw_bitmap_is_the_key_colour() {
    let palette = [[0, 0, 0, 0], [255, 0, 255, 255], [9, 8, 7, 255]];
    let table = [[0, 0, 0, 0], MAGENTA_ENTRY_16, [0x900, 0x800, 0x700, 0]];
    let raw = sprite_texels(&bitmap(1, vec![1, 2]), &palette, &table);
    assert_eq!(raw, vec![0, 0, 0, 0, 9, 8, 7, 255]);
    let rle = sprite_texels(&bitmap(0x44, vec![1, 2]), &palette, &table);
    assert_eq!(&rle[..4], &[255, 0, 255, 255]);
    let other = [[0, 0, 0, 0], [1, 2, 3, 0], [0x900, 0x800, 0x700, 0]];
    let kept = sprite_texels(&bitmap(1, vec![1, 2]), &palette, &other);
    assert_eq!(&kept[..4], &[255, 0, 255, 255]);
}

#[test]
fn a_raw_power_of_two_bitmap_loses_the_colour_opacity_flag() {
    let palette = [[0, 0, 255, 0], [0, 0, 0, 255], [121, 110, 71, 255]];
    let table = [[0, 0, 0xffff, 0], [0, 0, 0, 0], [0x7979, 0x6e6e, 0x4747, 0]];
    let wire = bitmap(0x803, vec![0, 1, 2, 1]);
    assert!(!keeps_color_alpha(&wire));
    let t = sprite_texels(&wire, &palette, &table);
    assert_eq!(t[0..4], [0, 0, 0, 0]);
    assert_eq!(t[4..8], [0, 0, 0, 255], "black is opaque");
    assert_eq!(t[8..12], [121, 110, 71, 255]);

    let wide = bitmap(0x802, vec![0, 1, 2]);
    assert!(keeps_color_alpha(&wide));
    let t = sprite_texels(&wide, &palette, &table);
    assert_eq!(t[4..8], [0, 0, 0, 0], "black has no opacity");
    assert_eq!(t[11], 0x79);
    let mut rle = bitmap(0x844, vec![0, 1, 2, 1]);
    assert!(keeps_color_alpha(&rle));
    rle.flags = 0x44;
    assert!(!keeps_color_alpha(&rle));
}
