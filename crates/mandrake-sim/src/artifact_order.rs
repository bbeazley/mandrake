use mandrake_common::game_options;
use std::collections::HashMap;

use mandrake_common::FourCC;
use mandrake_tag::{MeshMapActions, MeshTag};

use crate::engine_rng::TagSource;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Class {
    Media,
    Group,
    Projectile,
    Artifact,
    Monster,
    Unit,
    Scenery,
}

impl Class {
    fn group(self) -> ([u8; 4], usize) {
        match self {
            Class::Media => (*b"medi", 0x58),
            Class::Group => (
                *b"prgr",
                mandrake_tag::projectile::PROJECTILE_GROUP_DEFINITION_SIZE,
            ),
            Class::Projectile => (
                *b"proj",
                mandrake_tag::projectile::PROJECTILE_DEFINITION_SIZE,
            ),
            Class::Artifact => (*b"arti", 0xa8),
            Class::Monster => (*b"mons", 0x224),
            Class::Unit => (*b"unit", 8),
            Class::Scenery => (*b"scen", 0x20),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Loaded,
    Failed,
}

fn word(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4)
        .map_or(u32::MAX, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn tag(v: u32) -> FourCC {
    FourCC::new(v.to_be_bytes())
}

fn named(v: u32) -> Option<FourCC> {
    (v != u32::MAX).then(|| tag(v))
}

fn named_nonzero(v: u32) -> Option<FourCC> {
    (v != u32::MAX && v != 0).then(|| tag(v))
}

struct Walk<'a> {
    tags: &'a dyn TagSource,
    gameplay: i32,
    state: HashMap<(Class, FourCC), State>,
    artifacts: Vec<(FourCC, Vec<u8>)>,
}

impl Walk<'_> {
    fn load(&mut self, class: Class, id: FourCC) -> bool {
        match self.state.get(&(class, id)) {
            Some(State::Loaded) => return true,
            Some(State::Failed) => return false,
            None => {}
        }
        let (group, size) = class.group();
        let Some(d) = self.tags.tag(group, id).filter(|d| d.len() >= size) else {
            return false;
        };
        self.state.insert((class, id), State::Loaded);
        if class == Class::Artifact {
            self.artifacts.push((id, d.clone()));
        }
        let ok = match class {
            Class::Media => self.media(&d),
            Class::Group => self.group(&d),
            Class::Projectile => self.projectile(&d),
            Class::Artifact => self.artifact(&d),
            Class::Monster => self.monster(&d),
            Class::Unit => self.load(Class::Monster, tag(word(&d, 0))),
            Class::Scenery => self.scenery(&d),
        };
        if !ok {
            self.state.insert((class, id), State::Failed);
        }
        ok
    }

    fn optional(&mut self, class: Class, id: Option<FourCC>) -> bool {
        id.is_none_or(|id| self.load(class, id))
    }

    fn present(&self, group: [u8; 4], id: Option<FourCC>) -> bool {
        id.is_none_or(|id| self.tags.tag(group, id).is_some())
    }

    fn media(&mut self, d: &[u8]) -> bool {
        let mut ok = true;
        for k in 0..mandrake_tag::media::MEDIA_PROJECTILE_GROUP_COUNT {
            ok &= self.optional(Class::Group, named(word(d, 0x18 + 4 * k)));
        }
        ok
    }

    fn group(&mut self, d: &[u8]) -> bool {
        if !self.present(*b"meef", named_nonzero(word(d, 0x08)))
            || !self.present(*b"lpgr", named_nonzero(word(d, 0x10)))
        {
            return false;
        }
        let count = d
            .get(4..6)
            .map_or(0, |b| i16::from_be_bytes([b[0], b[1]]))
            .max(0) as usize;
        for k in 0..count.min(d.len().saturating_sub(0x20) / 0x20) {
            if !self.load(Class::Projectile, tag(word(d, 0x20 + 0x20 * k))) {
                return false;
            }
            self.optional(Class::Projectile, named_nonzero(word(d, 0x38 + 0x20 * k)));
        }
        true
    }

    fn projectile(&mut self, d: &[u8]) -> bool {
        if !self.present(*b"obje", Some(tag(word(d, 0x1c)))) {
            return false;
        }
        let mut ok = self.optional(Class::Projectile, named(word(d, 0x14)));
        ok &= self.optional(Class::Group, named(word(d, 0x10)));
        ok &= self.optional(Class::Artifact, named_nonzero(word(d, 0x78)));
        ok &= self.optional(Class::Projectile, named(word(d, 0x5c)));
        ok &= self.optional(Class::Group, named(word(d, 0x60)));
        ok &= self.optional(Class::Group, named_nonzero(word(d, 0x7c)));
        ok && self.optional(Class::Group, named_nonzero(word(d, 0x80)))
    }

    fn artifact(&mut self, d: &[u8]) -> bool {
        let flags = word(d, 0);
        let new = self.gameplay >= 0x6b9;
        if new && flags & 0x2_0000 != 0 {
            for at in [0x1c, 0x20, 0x24, 0x28] {
                if !self.optional(Class::Projectile, named_nonzero(word(d, at))) {
                    return false;
                }
            }
        }
        if flags & 4 != 0
            && let Some(m) = named(word(d, 0x98))
        {
            let mut monster = m;
            if new && flags & 0x8_0000 != 0 && self.load(Class::Unit, m) {
                let (group, _) = Class::Unit.group();
                if let Some(u) = self.tags.tag(group, m) {
                    monster = tag(word(&u, 0));
                }
            }
            self.load(Class::Monster, monster);
        }
        if flags & 0x100 != 0
            && self.gameplay > game_options::GAMEPLAY_1_5_1
            && let Some(m) = named(word(d, 0x04))
        {
            self.load(Class::Monster, m);
        }
        self.optional(Class::Projectile, named(word(d, 0x58)))
    }

    fn monster(&mut self, d: &[u8]) -> bool {
        let attacks = d
            .get(0x88..0x8a)
            .map_or(0, |b| i16::from_be_bytes([b[0], b[1]]))
            .clamp(0, 4) as usize;
        for k in 0..attacks {
            if !self.optional(Class::Projectile, named(word(d, 0x8c + 0x40 * k + 4))) {
                return false;
            }
        }
        let chain = [
            (Class::Group, named(word(d, 0x194))),
            (Class::Group, named(word(d, 0x1a4))),
            (Class::Group, named(word(d, 0x3c))),
            (Class::Group, named(word(d, 0x1a0))),
            (Class::Group, named(word(d, 0x1e8))),
            (Class::Group, named(word(d, 0x1e4))),
            (Class::Group, named_nonzero(word(d, 0x204))),
            (Class::Group, named_nonzero(word(d, 0x210))),
            (Class::Group, named_nonzero(word(d, 0x220))),
            (Class::Projectile, named(word(d, 0x1ec))),
        ];
        chain.into_iter().all(|(c, id)| self.optional(c, id))
    }

    fn scenery(&mut self, d: &[u8]) -> bool {
        let mut ok = self.optional(Class::Projectile, named(word(d, 0x0c)));
        ok &= self.optional(Class::Group, named(word(d, 0x18)));
        ok & self.optional(Class::Group, named(word(d, 0x1c)))
    }
}

pub(crate) fn artifact_load_order(
    mesh: &MeshTag,
    actions: Option<&MeshMapActions>,
    tags: &dyn TagSource,
    gameplay: i32,
    player_teams: &[i16],
    internal_projectiles: &[[u8; 4]],
    internal_groups: &[[u8; 4]],
) -> Vec<(FourCC, Vec<u8>)> {
    let mut w = Walk {
        tags,
        gameplay,
        state: HashMap::new(),
        artifacts: Vec::new(),
    };
    w.optional(Class::Media, named(u32::from_be_bytes(mesh.media_tag.0)));
    for &p in internal_projectiles {
        w.optional(Class::Projectile, named(u32::from_be_bytes(p)));
    }
    for &g in internal_groups {
        w.optional(Class::Group, named(u32::from_be_bytes(g)));
    }
    for p in actions
        .iter()
        .flat_map(|a| &a.actions)
        .flat_map(|r| &r.parameters)
    {
        let class = match p.kind {
            9 => Class::Group,
            12 => Class::Projectile,
            _ => continue,
        };
        for &v in &p.values {
            w.optional(class, named(v as u32));
        }
    }
    for kind in [1, 3, 9] {
        for e in mesh.palettes.iter().filter(|e| e.marker_type == kind) {
            match kind {
                1 => {
                    w.load(Class::Scenery, e.tag_id);
                }
                3 => {
                    let team = e.team_index();
                    let loaded = e.flags & 0x10 != 0
                        || (team != -1
                            && (gameplay > game_options::GAMEPLAY_1_7_2
                                || player_teams.contains(&team)));
                    if loaded {
                        w.load(Class::Unit, e.tag_id);
                    }
                }
                _ => {
                    w.load(Class::Projectile, e.tag_id);
                }
            }
        }
    }
    w.artifacts
}
