use crate::args::{self, Args, TflExtras};
use crate::ui_scale::{Sampling, UiFilter};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ViewSettings {
    pub(crate) sampling: Sampling,
    pub(crate) sprite_filter: Option<UiFilter>,
    pub(crate) ui_filter: Option<UiFilter>,
    pub(crate) reflection_soft_edge: bool,
    pub(crate) extras: TflExtras,
    pub(crate) water: Option<args::Water>,
    pub(crate) water_maps: Vec<String>,
    pub(crate) detail_textures: bool,
    pub(crate) translucent_bars: bool,
    pub(crate) smoothing: bool,
    pub(crate) sprite_interpolation: bool,
    pub(crate) hover_frames: bool,
    pub(crate) sound_rules: crate::tfl_sound::TflSoundRules,
    pub(crate) tfl_selection_boxes: bool,
}

fn tfl_switch(
    load: fn(&std::path::Path) -> Result<Option<bool>, String>,
    fallback: &str,
) -> Option<bool> {
    crate::settings::user_settings_path().and_then(|path| {
        load(&path).unwrap_or_else(|e| {
            tracing::warn!("settings: {e}; {fallback}");
            None
        })
    })
}

impl ViewSettings {
    pub(crate) fn of(a: &Args) -> Self {
        Self {
            sampling: a.sampling,
            sprite_filter: a.sprite_filter,
            ui_filter: a.ui_filter,
            reflection_soft_edge: args::reflection_soft_edge_enabled(
                a.reflection_soft_edge,
                a.sampling,
            ),
            extras: TflExtras::of(a),
            water: a.water,
            water_maps: a.water_maps.clone(),
            detail_textures: crate::startup::detail_textures_wanted(a),
            translucent_bars: a.translucent_bars,
            smoothing: tfl_switch(crate::settings::load_tfl_smoothing, "TFL is drawn smoothed")
                .unwrap_or(true),
            sprite_interpolation: crate::sprite_interp::start_state(
                tfl_switch(
                    crate::settings::load_tfl_sprite_interpolation,
                    "TFL's in-betweens are drawn",
                ),
                true,
            ),
            hover_frames: tfl_switch(
                crate::settings::load_tfl_hover_frames,
                "TFL frames the monster under the pointer",
            )
            .unwrap_or(true),
            sound_rules: crate::settings::user_settings_path()
                .and_then(|path| {
                    crate::settings::load_tfl_sound_rules(&path).unwrap_or_else(|e| {
                        tracing::warn!("settings: {e}; TFL's sound mixes by Myth II's rules");
                        None
                    })
                })
                .unwrap_or_default(),
            tfl_selection_boxes: a.tfl_selection_boxes,
        }
    }

    pub(crate) fn water_on(&self, mesh: &str) -> bool {
        args::water_on(self.water, &self.water_maps, mesh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn of_follows_the_flags() {
        let a = Args::parse_from(["mandrake"]);
        let v = ViewSettings::of(&a);
        assert!(v.reflection_soft_edge, "on by default");
        assert!(v.extras.fog && v.extras.lighting && v.extras.detail_shadows);
        let a = Args::parse_from(["mandrake", "--sampling", "linear"]);
        let v = ViewSettings::of(&a);
        assert!(!v.reflection_soft_edge, "linear is the engine's picture");
        let a = Args::parse_from([
            "mandrake",
            "--water",
            "enhanced",
            "--water-maps",
            "01bd",
            "--reflection-soft-edge",
            "off",
        ]);
        let v = ViewSettings::of(&a);
        assert!(!v.reflection_soft_edge);
        assert!(v.water_on("01bd"));
        assert!(!v.water_on("le3e"));
        let a = Args::parse_from(["mandrake", "--water", "enhanced"]);
        assert!(
            ViewSettings::of(&a).water_on("le3e"),
            "every map without --water-maps"
        );
    }
}
