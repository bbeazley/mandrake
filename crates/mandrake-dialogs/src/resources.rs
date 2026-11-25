use std::collections::HashMap;
use std::sync::Arc;

use mandrake_common::FourCC;
use mandrake_session::MenuTrack;
use mandrake_tag::font::{self, FontFamily, RgbColor, TextLocale};
use mandrake_tag::interface::{
    INTERFACE_FONT_COUNT, InterfaceDefinition, STANDARD_INTERFACE_DEFINITION,
};
use mandrake_tag::sound_manager::LocalRandom;
use mandrake_tag::{DialogDefinition, TagLookup};
use mandrake_view::interface_bitmap::InterfaceArt;

pub const BAIE: FourCC = FourCC(*b"baie");
const DIALOG_COLORS_SEQUENCE: i32 = 13;

type TagSource = Box<dyn Fn(FourCC, FourCC) -> Option<Vec<u8>>>;
type BesideSource = Box<dyn Fn(FourCC, FourCC, (FourCC, FourCC)) -> Option<Vec<u8>>>;
type BesideArt = HashMap<(FourCC, (FourCC, FourCC)), Option<Arc<InterfaceArt>>>;
pub type CampaignArtSource = Box<dyn Fn(&[String], FourCC) -> Option<Vec<u8>>>;
pub type WallClockSource = Box<dyn Fn() -> i64>;

pub struct Resources {
    source: TagSource,
    beside: Option<BesideSource>,
    beside_art: BesideArt,
    campaign_art: Option<CampaignArtSource>,
    plugin_art: HashMap<(Vec<String>, FourCC), Option<Arc<InterfaceArt>>>,
    art: HashMap<FourCC, Option<Arc<InterfaceArt>>>,
    strings: HashMap<FourCC, Vec<Vec<u8>>>,
    fonts: [Option<Arc<FontFamily>>; INTERFACE_FONT_COUNT],
    locale: TextLocale,
    random: LocalRandom,
    last_track: Option<FourCC>,
    edit_ring: Option<RgbColor>,
    wall_clock: Option<WallClockSource>,
}

impl std::fmt::Debug for Resources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resources")
            .field("art", &self.art.len())
            .finish_non_exhaustive()
    }
}

impl Resources {
    pub fn new(tags: TagLookup) -> Self {
        let tags = Arc::new(tags);
        let beside = tags.clone();
        let mut res = Self::from_source(move |group, id| tags.get(group, id));
        res.beside = Some(Box::new(move |group, id, near| {
            beside.get_beside(group, id, near)
        }));
        res
    }

    pub fn from_source(source: impl Fn(FourCC, FourCC) -> Option<Vec<u8>> + 'static) -> Self {
        let inte = source(FourCC(*b"inte"), STANDARD_INTERFACE_DEFINITION)
            .and_then(|d| InterfaceDefinition::from_bytes(&d).ok());
        let fonts = std::array::from_fn(|i| {
            let id = inte.as_ref()?.font(i)?;
            let mut family = FontFamily::load(id, |id| source(FourCC(*b"font"), id)).ok()?;
            if i == 0 {
                family.adjust_line_height(-2);
            }
            Some(Arc::new(family))
        });
        let locale = source(FourCC(*b"stli"), font::TEXT_LOCALE_ID).map_or_else(
            || TextLocale::from_stli_bytes(&[]),
            |d| TextLocale::from_stli_bytes(&d),
        );
        Self {
            source: Box::new(source),
            beside: None,
            beside_art: HashMap::new(),
            campaign_art: None,
            plugin_art: HashMap::new(),
            art: HashMap::new(),
            strings: HashMap::new(),
            fonts,
            locale,
            random: LocalRandom::new(0),
            last_track: None,
            edit_ring: None,
            wall_clock: None,
        }
    }

    pub fn set_random_seed(&mut self, seed: u32) {
        self.random = LocalRandom::new(seed);
    }

    pub fn next_menu_track(&mut self, tracks: &[MenuTrack]) -> Option<FourCC> {
        let roll = self.random.next_u16();
        let tag = pick_menu_track(tracks, self.last_track, roll)?;
        self.last_track = Some(tag);
        Some(tag)
    }

    pub fn art(&mut self, id: FourCC) -> Option<Arc<InterfaceArt>> {
        let source = &self.source;
        self.art
            .entry(id)
            .or_insert_with(|| {
                let art = source(FourCC(*b".256"), id)
                    .and_then(InterfaceArt::from_bytes)
                    .map(Arc::new);
                if art.is_none() {
                    tracing::warn!(%id, "dialogs: no .256 tag");
                }
                art
            })
            .clone()
    }

    pub fn art_beside(&mut self, id: FourCC, near: (FourCC, FourCC)) -> Option<Arc<InterfaceArt>> {
        let Some(beside) = &self.beside else {
            return self.art(id);
        };
        self.beside_art
            .entry((id, near))
            .or_insert_with(|| {
                beside(FourCC(*b".256"), id, near)
                    .and_then(InterfaceArt::from_bytes)
                    .map(Arc::new)
            })
            .clone()
    }

    pub fn set_campaign_art(&mut self, source: CampaignArtSource) {
        self.campaign_art = Some(source);
    }

    pub fn art_of_plugins(&mut self, plugins: &[String], id: FourCC) -> Option<Arc<InterfaceArt>> {
        if plugins.is_empty() {
            return self.art(id);
        }
        let Some(source) = &self.campaign_art else {
            return self.art(id);
        };
        self.plugin_art
            .entry((plugins.to_vec(), id))
            .or_insert_with(|| {
                source(plugins, id)
                    .and_then(InterfaceArt::from_bytes)
                    .map(Arc::new)
            })
            .clone()
    }

    pub fn dialog(&self, id: FourCC) -> Option<DialogDefinition> {
        let Some(bytes) = (self.source)(FourCC(*b"bina"), id) else {
            tracing::warn!(%id, "dialogs: no bina tag");
            return None;
        };
        let mut def = match DialogDefinition::from_bytes(&bytes) {
            Ok(def) => def,
            Err(e) => {
                tracing::warn!(%id, "dialogs: {e}");
                return None;
            }
        };
        if let Some(ditl) = (self.source)(FourCC(*b"ditl"), id) {
            def.apply_ditl(&ditl);
        }
        Some(def)
    }

    pub fn tag(&self, group: FourCC, id: FourCC) -> Option<Vec<u8>> {
        (self.source)(group, id)
    }

    pub fn string_list(&mut self, id: FourCC) -> &[Vec<u8>] {
        let source = &self.source;
        self.strings
            .entry(id)
            .or_insert_with(|| {
                source(FourCC(*b"stli"), id)
                    .map(|d| mandrake_tag::mesh_links::string_list_lines(&d))
                    .unwrap_or_default()
            })
            .as_slice()
    }

    pub fn string_line(&mut self, id: FourCC, index: usize) -> &[u8] {
        self.string_list(id).get(index).map_or(&[], Vec::as_slice)
    }

    pub fn set_wall_clock(&mut self, clock: WallClockSource) {
        self.wall_clock = Some(clock);
    }

    pub fn unix_secs(&self, fallback_ms: u64) -> i64 {
        if let Some(clock) = &self.wall_clock {
            return clock();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = fallback_ms;
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64)
        }
        #[cfg(target_arch = "wasm32")]
        {
            (fallback_ms / 1000) as i64
        }
    }

    pub fn font(&self, index: usize) -> Option<&Arc<FontFamily>> {
        self.fonts.get(index)?.as_ref()
    }

    pub fn locale(&self) -> &TextLocale {
        &self.locale
    }

    pub fn dialog_color(&mut self, entry: i32) -> RgbColor {
        self.art(BAIE).map_or([0xffff; 3], |a| {
            mandrake_tag::hud::rgb_color_from_sequence(
                &a.raw,
                &a.coll,
                DIALOG_COLORS_SEQUENCE,
                entry,
            )
        })
    }

    pub fn edit_ring(&mut self) -> RgbColor {
        self.edit_ring.unwrap_or_else(|| self.dialog_color(9))
    }

    pub fn set_edit_ring(&mut self, color: RgbColor) {
        self.edit_ring = Some(color);
    }
}

pub fn pick_menu_track(tracks: &[MenuTrack], last: Option<FourCC>, roll: u16) -> Option<FourCC> {
    let candidates = || tracks.iter().filter(|t| Some(t.tag) != last);
    let total: i32 = candidates().map(|t| i32::from(t.weight)).sum();
    if total <= 0 {
        return tracks.first().map(|t| t.tag);
    }
    let mut pick = (total * i32::from(roll)) >> 16;
    for track in candidates() {
        pick -= i32::from(track.weight);
        if pick < 0 {
            return Some(track.tag);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACKS: [MenuTrack; 4] = [
        MenuTrack {
            tag: FourCC(*b"mamm"),
            weight: 20,
        },
        MenuTrack {
            tag: FourCC(*b"mamn"),
            weight: 20,
        },
        MenuTrack {
            tag: FourCC(*b"mamo"),
            weight: 10,
        },
        MenuTrack {
            tag: FourCC(*b"mamp"),
            weight: 20,
        },
    ];

    fn tag(t: &[u8; 4]) -> Option<FourCC> {
        Some(FourCC(*t))
    }

    #[test]
    fn the_pick_goes_by_weight_and_never_repeats() {
        let roll = |at: i32, total: i32| ((at << 16) / total + 1) as u16;
        assert_eq!(pick_menu_track(&TRACKS, None, 0), tag(b"mamm"));
        assert_eq!(pick_menu_track(&TRACKS, None, roll(20, 70)), tag(b"mamn"));
        assert_eq!(pick_menu_track(&TRACKS, None, roll(40, 70)), tag(b"mamo"));
        assert_eq!(pick_menu_track(&TRACKS, None, roll(50, 70)), tag(b"mamp"));
        assert_eq!(pick_menu_track(&TRACKS, None, u16::MAX), tag(b"mamp"));
        assert_eq!(pick_menu_track(&TRACKS, tag(b"mamm"), 0), tag(b"mamn"));
        assert_eq!(
            pick_menu_track(&TRACKS, tag(b"mamm"), roll(20, 50)),
            tag(b"mamo")
        );
        assert_eq!(
            pick_menu_track(&TRACKS, tag(b"mamo"), roll(40, 60)),
            tag(b"mamp")
        );
        let mut res = Resources::from_source(|_, _| None);
        res.set_random_seed(0x1234_5678);
        let mut last = None;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let next = res.next_menu_track(&TRACKS);
            assert_ne!(next, last, "never the last");
            seen.insert(next);
            last = next;
        }
        assert_eq!(seen.len(), 4, "all of them come up");
    }

    #[test]
    fn one_track_plays_again_and_none_plays_nothing() {
        let one = &TRACKS[..1];
        assert_eq!(pick_menu_track(one, tag(b"mamm"), 0), tag(b"mamm"));
        assert_eq!(pick_menu_track(&[], None, 0), None);
    }

    #[test]
    fn string_lists_are_cached_after_first_read() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let reads = Arc::new(AtomicUsize::new(0));
        let counter = reads.clone();
        let mut res = Resources::from_source(move |group, id| {
            if group == FourCC(*b"stli") && id == FourCC(*b"inmn") {
                counter.fetch_add(1, Ordering::Relaxed);
                return Some(b"first\rsecond\0".to_vec());
            }
            None
        });
        for _ in 0..100 {
            assert_eq!(res.string_line(FourCC(*b"inmn"), 0), b"first");
            assert_eq!(res.string_line(FourCC(*b"inmn"), 1), b"second");
        }
        assert_eq!(reads.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn wall_clock_hook_overrides_unix_secs() {
        let mut res = Resources::from_source(|_, _| None);
        res.set_wall_clock(Box::new(|| 1_700_000_000));
        assert_eq!(res.unix_secs(12_345), 1_700_000_000);
    }
}
