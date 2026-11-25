use crate::corpse;
use crate::sprite_manager::ClientSpriteManager;
use mandrake_common::FourCC;
use mandrake_tag::sound_manager::LocalRandom;
use mandrake_view::landscape::LandscapeTexels;

pub(crate) fn stain_mirror(random: &mut LocalRandom) -> (bool, bool) {
    let x = random.next_u16() & 1 != 0;
    let y = random.next_u16() & 1 != 0;
    (x, y)
}

pub(crate) fn apply_landscape_edit(
    sprite_mgr: &ClientSpriteManager,
    texels: &mut LandscapeTexels,
    edit: &corpse::LandscapeEdit,
    random: &mut LocalRandom,
) -> Option<mandrake_view::landscape::PixelRect> {
    match edit {
        corpse::LandscapeEdit::Stain(s) => {
            let mirror = stain_mirror(random);
            apply_stain(
                sprite_mgr,
                texels,
                (s.collection, s.sequence),
                (s.point[0], s.point[1]),
                s.random,
                mirror,
            )
        }
        corpse::LandscapeEdit::Bloody { point, .. } => {
            let media = sprite_mgr.scenery_objects.media.as_deref();
            let is_media = |x: i32, y: i32| media.is_some_and(|m| m.coverage.pixel_is_media(x, y));
            texels.bloody_mesh(*point, &is_media, &mut || random.next_u16())
        }
        corpse::LandscapeEdit::Footstep { point } => {
            texels.footstep(*point, &mut || random.next_u16())
        }
    }
}

pub(crate) fn apply_stain(
    sprite_mgr: &ClientSpriteManager,
    texels: &mut LandscapeTexels,
    sequence: (FourCC, i16),
    point: (i32, i32),
    random: u16,
    mirror: (bool, bool),
) -> Option<mandrake_view::landscape::PixelRect> {
    let stain = sprite_mgr.corpse_sprites.stains.get(&sequence)?;
    let bitmap = stain.bitmap(random)?;
    let media = sprite_mgr.scenery_objects.media.as_deref();
    let is_media = |x: i32, y: i32| media.is_some_and(|m| m.coverage.pixel_is_media(x, y));
    texels.apply_stain(bitmap, &stain.color_table, point, mirror, &is_media)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stain_turns_as_the_engine_does_from_the_same_state() {
        for (state, mirror, after) in [
            (2788669655, (false, false), 3364778785),
            (4214784134, (false, true), 2501716392),
            (2360904995, (true, true), 2223368525),
        ] {
            let mut random = LocalRandom::new(state);
            assert_eq!(stain_mirror(&mut random), mirror);
            assert_eq!(random.seed, after);
        }
    }
}
