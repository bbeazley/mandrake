use mandrake_tag::MeshTag;
use mandrake_tag::model_hit::ModelPlacement;
use mandrake_view::model_surfaces::EngineGeometry;
use mandrake_view::view_region::ViewRegion;

use super::WorldView;
use crate::media::MapMedia;
use crate::projfx_log::ProjfxLog;
use crate::selection_input::MonsterPick;
use crate::sprite_manager::ClientSpriteManager;
use crate::unit_anim::UnitAnimator;

pub(crate) trait PickSim {
    fn could_pick_up(&self, selection: &[u32], object: i32) -> Option<i16>;
}

pub(crate) trait Myth2PickSim: PickSim {
    fn drawn_units(&self, animator: &UnitAnimator, sprite_mgr: &ClientSpriteManager) -> WorldView;
    fn drawn_objects(&self, fraction: i32, trace: Option<&ProjfxLog>) -> WorldView;
    fn monster_pick(&self, id: u32, selected: bool) -> MonsterPick;
    fn under_media(&self, id: u32, margin: i32) -> Option<bool>;
    fn band_eligible(&self, id: u32) -> bool;
    fn same_type_flood(&self, base: u32) -> Vec<u32>;
    fn all_controlled(&self, selection: &[u32]) -> bool;
    fn special_against_friendlies(&self, selection: &[u32]) -> Vec<u32>;
    fn unit_location(&self, id: u32) -> Option<(i32, i32)>;
    fn command_point(&self, p: [f32; 3]) -> (i32, i32);
    fn drawn_models<'a>(
        &'a self,
        region: &ViewRegion,
        map: &'a MapMedia,
        mesh_tag: &MeshTag,
    ) -> Vec<(&'a EngineGeometry, &'a ModelPlacement)>;
}
