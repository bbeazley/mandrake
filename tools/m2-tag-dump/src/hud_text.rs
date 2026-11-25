use mandrake_common::FourCC;
use mandrake_tag::font::{self, Bitmap32, FontFamily, RgbColor, TextClip, TextLocale, text_flags};
use mandrake_tag::hud::{self, status_item, status_sequence};
use mandrake_tag::interface::{FONT_SYSTEM, InterfaceDefinition, STANDARD_INTERFACE_DEFINITION};
use mandrake_tag::{CollectionTag, DialogDefinition, Rect, StringListTag, TagLookup};

pub struct TitleTags {
    pub family: FontFamily,
    pub locale: TextLocale,
    pub status_dialog: DialogDefinition,
    pub status_art: CollectionTag,
    pub status_art_raw: Vec<u8>,
    pub difficulty_names: StringListTag,
}

fn fc(s: &[u8; 4]) -> FourCC {
    FourCC::new(*s)
}

impl TitleTags {
    pub fn load(tags: &TagLookup) -> anyhow::Result<Self> {
        let get = |g: &[u8; 4], id: FourCC| {
            tags.get(fc(g), id)
                .ok_or_else(|| anyhow::anyhow!("tag {} {id} missing", String::from_utf8_lossy(g)))
        };
        let inte = InterfaceDefinition::from_bytes(&get(b"inte", STANDARD_INTERFACE_DEFINITION)?)?;
        let font_id = inte
            .font(FONT_SYSTEM)
            .ok_or_else(|| anyhow::anyhow!("inte: no font 0"))?;
        let family = FontFamily::load(font_id, |id| tags.get(fc(b"font"), id))?;
        let locale = TextLocale::from_stli_bytes(&get(b"stli", font::TEXT_LOCALE_ID)?);
        let art = get(b".256", hud::STATUS_BAR_ID)?;
        Ok(Self {
            family,
            locale,
            status_dialog: DialogDefinition::from_bytes(&get(b"bina", hud::STATUS_BAR_ID)?)?,
            status_art: CollectionTag::from_bytes(&art)?,
            status_art_raw: art,
            difficulty_names: StringListTag::from_bytes(&get(b"stli", hud::DIFFICULTY_NAMES_ID)?)?,
        })
    }

    pub fn swatch(&self, entry: i32) -> RgbColor {
        hud::rgb_color_from_sequence(
            &self.status_art_raw,
            &self.status_art,
            status_sequence::COLOR_SWATCH as i32,
            entry,
        )
    }

    pub fn fill_tile(&self) -> Option<&mandrake_tag::BitmapFrame> {
        let coll = &self.status_art;
        coll.sequence(status_sequence::BACKGROUND)
            .and_then(|s| s.bitmap_index(0, 0))
            .and_then(|b| coll.frame(b))
    }
}

pub fn title_text(tags: &TitleTags, scoring: &str, difficulty: usize) -> Vec<u8> {
    let d = tags.difficulty_names.get(difficulty).unwrap_or("");
    "%s  (%s)"
        .replacen("%s", scoring, 1)
        .replacen("%s", d, 1)
        .into_bytes()
}

pub fn render_title(tags: &TitleTags, text: &[u8], screen_w: i32) -> (Bitmap32, Rect) {
    let h = tags.status_dialog.height();
    let mut bmp = Bitmap32::new(screen_w, h, 0xff00_0000);
    if let Some(tile) = tags.fill_tile() {
        let pal = tags.status_art.palette();
        let (tw, th) = (tile.width as i32, tile.height as i32);
        for y in 0..h {
            for x in 0..screen_w {
                let i = tile.pixels[((y % th) * tw + x % tw) as usize] as usize;
                let c = pal.get(i).copied().unwrap_or([0; 4]);
                bmp.set(
                    x,
                    y,
                    0xff00_0000 | (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32,
                );
            }
        }
    }
    let item = tags
        .status_dialog
        .item(status_item::TITLE)
        .map(|i| i.rect)
        .unwrap_or_default();
    let rect = hud::status_bar_item_rect(status_item::TITLE, item, screen_w);
    font::draw_string(
        &mut bmp,
        rect,
        TextClip::SameAsRect,
        &tags.family,
        font::STYLE_BOLD,
        text,
        tags.swatch(2),
        text_flags::SHADOW,
        0,
        &tags.locale,
    );
    (bmp, rect)
}

pub fn title_crop(tags: &TitleTags, text: &[u8], rect: Rect) -> Rect {
    let m = tags.family.base.margin();
    let w = font::string_width(&tags.family, font::STYLE_BOLD, text, &tags.locale);
    Rect::new(rect.top, rect.left + m, rect.bottom, rect.left + m + w + 2)
}

pub fn compare(bmp: &Bitmap32, img: &[u8], img_w: i32, crop: Rect) -> (usize, usize, u8, f64) {
    let (mut n, mut exact, mut worst, mut se) = (0usize, 0usize, 0u8, 0f64);
    for y in crop.top..crop.bottom {
        for x in crop.left..crop.right {
            let p = bmp.get(x, y);
            let ours = [(p >> 16) as u8, (p >> 8) as u8, p as u8];
            let o = ((y * img_w + x) * 3) as usize;
            let theirs = &img[o..o + 3];
            let d = (0..3)
                .map(|k| ours[k].abs_diff(theirs[k]))
                .max()
                .unwrap_or(0);
            for k in 0..3 {
                se += (ours[k] as f64 - theirs[k] as f64).powi(2);
            }
            worst = worst.max(d);
            exact += (d == 0) as usize;
            n += 1;
        }
    }
    let mse = se / (n.max(1) * 3) as f64;
    let psnr = if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0f64 * 255.0 / mse).log10()
    };
    (n, exact, worst, psnr)
}

pub fn hud_title(
    tags: &TagLookup,
    reference: &std::path::Path,
    scoring: &str,
    difficulty: usize,
    out: Option<&std::path::Path>,
    zoom: u32,
) -> anyhow::Result<()> {
    let t = TitleTags::load(tags)?;
    let img = image::open(reference)?.to_rgb8();
    let text = title_text(&t, scoring, difficulty);
    let (bmp, rect) = render_title(&t, &text, img.width() as i32);
    let crop = title_crop(&t, &text, rect);
    let (n, exact, worst, psnr) = compare(&bmp, img.as_raw(), img.width() as i32, crop);
    println!(
        "title {:?} font {} style 1, colour {:04x?}, item 8 {rect}, crop {crop}: {n} pixels, {exact} exact, largest difference {worst}, PSNR {psnr:.2} dB",
        String::from_utf8_lossy(&text),
        t.family.id,
        t.swatch(2)
    );
    if let Some(out) = out {
        let (cw, ch) = (crop.width() as u32, crop.height() as u32);
        let mut strip = image::RgbImage::new(cw * zoom, ch * zoom * 3);
        for y in 0..ch {
            for x in 0..cw {
                let (sx, sy) = (crop.left + x as i32, crop.top + y as i32);
                let o = img.get_pixel(sx as u32, sy as u32).0;
                let p = bmp.get(sx, sy);
                let us = [(p >> 16) as u8, (p >> 8) as u8, p as u8];
                let d: [u8; 3] = std::array::from_fn(|k| o[k].abs_diff(us[k]).saturating_mul(8));
                for (band, c) in [o, us, d].into_iter().enumerate() {
                    for dy in 0..zoom {
                        for dx in 0..zoom {
                            strip.put_pixel(
                                x * zoom + dx,
                                (band as u32 * ch + y) * zoom + dy,
                                image::Rgb(c),
                            );
                        }
                    }
                }
            }
        }
        strip.save(out)?;
        println!("wrote {}", out.display());
    }
    Ok(())
}
