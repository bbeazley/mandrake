use std::collections::HashSet;

use mandrake_common::FourCC;
use mandrake_tag::mesh::MeshTag;
use mandrake_tag::mesh_links::{
    CachedEntry, CampaignTags, TagSearch, string_list_lines, text_bytes,
};

use crate::rules::{ENTRY_LISTED, ENTRY_SOLO};
use crate::screen::{Difficulty, MenuTrack, Narration};
use crate::text::Text;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Catalog {
    pub campaigns: Vec<Campaign>,
    pub levels: Vec<Level>,
    pub difficulties: Vec<Difficulty>,
    pub hint_tabs: Vec<Text>,
    pub general_hints: Vec<Text>,
    pub menu_music: Vec<MenuTrack>,
    pub campaign_end: Narration,
    pub net_mapsets: Vec<Campaign>,
    pub net_max_teams: Vec<MaxTeams>,
    pub listed_plugins: Vec<i32>,
}

pub type MaxTeams = ((i32, FourCC), i16);

pub const ENTRY_NET_MASK: u32 = 0xffff;

impl Catalog {
    pub fn level(&self, mesh: FourCC) -> Option<&Level> {
        self.levels.iter().find(|l| l.mesh == mesh)
    }

    pub fn level_of_campaign(&self, mesh: FourCC, plugin_checksum: i32) -> Option<&Level> {
        self.levels
            .iter()
            .find(|l| l.mesh == mesh && l.plugin_checksum == plugin_checksum)
    }

    pub fn from_tags(tags: &CampaignTags) -> Self {
        let base = tags.base_search();
        let mut levels = Vec::new();
        let mut campaigns = vec![build_campaign(
            b"Myth II: Soulblighter",
            NO_PLUGIN,
            Vec::new(),
            &tags.base_entries,
            &base,
            &mut levels,
        )];

        let mut plugins: Vec<(usize, usize)> = tags
            .plugins
            .iter()
            .enumerate()
            .map(|(i, plugin)| {
                (
                    i,
                    plugin.entries.iter().filter(|e| in_solo_count(e)).count(),
                )
            })
            .filter(|&(_, count)| count > 0)
            .collect();
        let name_key = |i: usize| tags.plugins[i].name.to_ascii_lowercase();
        plugins.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| name_key(a.0).cmp(&name_key(b.0)))
        });
        for (index, _) in plugins {
            let plugin = &tags.plugins[index];
            campaigns.push(build_campaign(
                plugin.name.as_bytes(),
                plugin.checksum,
                tags.plugin_names(index),
                &plugin.entries,
                &tags.plugin_search(index),
                &mut levels,
            ));
        }

        let (net_mapsets, net_max_teams) = net_mapsets(tags);
        let listed_plugins = tags
            .plugins
            .iter()
            .filter(|p| p.entries.iter().any(|e| e.flags & ENTRY_LISTED != 0))
            .map(|p| p.checksum)
            .collect();
        let mut ingt = string_list(&base, Some(INGT));
        let general_hints = if ingt.len() > 2 {
            ingt.split_off(2)
        } else {
            Vec::new()
        };
        let hint_tabs = ingt;
        Self {
            net_mapsets,
            net_max_teams,
            listed_plugins,
            campaigns,
            levels,
            difficulties: read_difficulties(&base),
            hint_tabs,
            general_hints,
            menu_music: MENU_MUSIC
                .into_iter()
                .filter(|track| base.has(SOUND, track.tag))
                .collect(),
            campaign_end: Narration {
                art: Some(CAMPAIGN_END_ART).filter(|&tag| base.has(COLLECTION, tag)),
                text: text_tag(&base, Some(CAMPAIGN_END_TEXT)).unwrap_or_default(),
                narration: Some(CAMPAIGN_END_SOUND).filter(|&tag| base.has(SOUND, tag)),
            },
        }
    }
}

fn mesh_max_teams(header: &[u8]) -> i16 {
    header
        .get(MESH_MAX_TEAMS..MESH_MAX_TEAMS + 4)
        .and_then(|b| b.try_into().ok())
        .map_or(0, |b| {
            i32::from_be_bytes(b).clamp(0, i32::from(i16::MAX)) as i16
        })
}

fn net_mapsets(tags: &CampaignTags) -> (Vec<Campaign>, Vec<MaxTeams>) {
    let in_net_count =
        |e: &CachedEntry| e.flags & ENTRY_NET_MASK != 0 && e.flags & ENTRY_LISTED != 0;
    let mapset =
        |name: &[u8], checksum: i32, plugins: Vec<String>, entries: &[CachedEntry]| Campaign {
            name: Text(name.to_vec()),
            checksum,
            plugins,
            entries: entries
                .iter()
                .map(|e| ListEntry {
                    mesh: e.mesh,
                    name: Text(e.name.clone()),
                    flags: e.flags,
                    order: e.order,
                    extra_flags: e.extra_flags,
                    plugin_checksum: checksum,
                })
                .collect(),
        };
    let mut teams = Vec::new();
    let mut max_teams = |search: &TagSearch<'_>, checksum: i32, entries: &[CachedEntry]| {
        for e in entries.iter().filter(|e| in_net_count(e)) {
            let most = search
                .mesh_header_bytes(e.mesh)
                .map_or(0, |h| mesh_max_teams(&h));
            teams.push(((checksum, e.mesh), most));
        }
    };
    max_teams(&tags.base_search(), NO_PLUGIN, &tags.base_entries);
    let mut base = tags.base_entries.clone();
    if let Some(index) = tags.plugins.iter().position(|p| p.name == DEFAULT_MAPS) {
        let entries = &tags.plugins[index].entries;
        max_teams(&tags.plugin_search(index), NO_PLUGIN, entries);
        base.extend(entries.iter().cloned());
    }
    let mut out = vec![mapset(
        b"Myth II: Soulblighter",
        NO_PLUGIN,
        Vec::new(),
        &base,
    )];
    let mut plugins: Vec<(usize, usize)> = tags
        .plugins
        .iter()
        .enumerate()
        .map(|(i, p)| (i, p.entries.iter().filter(|e| in_net_count(e)).count()))
        .filter(|&(i, count)| count > 0 && !NOT_MAPSETS.contains(&tags.plugins[i].name.as_str()))
        .collect();
    let name_key = |i: usize| tags.plugins[i].name.to_ascii_lowercase();
    plugins.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| name_key(a.0).cmp(&name_key(b.0)))
    });
    for (index, _) in plugins {
        let plugin = &tags.plugins[index];
        max_teams(&tags.plugin_search(index), plugin.checksum, &plugin.entries);
        out.push(mapset(
            plugin.name.as_bytes(),
            plugin.checksum,
            tags.plugin_names(index),
            &plugin.entries,
        ));
    }
    (out, teams)
}

const DEFAULT_MAPS: &str = "Patch 1.8.5 Maps";

const NOT_MAPSETS: [&str; 3] = [DEFAULT_MAPS, "Patch 1.5 Unity", "Patch 1.8.5 Unity"];

const MESH_MAX_TEAMS: usize = 0x54;

fn in_solo_count(entry: &CachedEntry) -> bool {
    let both = ENTRY_SOLO | ENTRY_LISTED;
    entry.flags & both == both
}

const NO_PLUGIN: i32 = -1;

const COLLECTION: FourCC = FourCC(*b".256");
const SOUND: FourCC = FourCC(*b"soun");
const AMBIENT: FourCC = FourCC(*b"amso");
const STRINGS: FourCC = FourCC(*b"stli");
const TEXT: FourCC = FourCC(*b"text");

pub const MENU_MUSIC: [MenuTrack; 4] = [
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
const PREVIEW_ART: FourCC = FourCC(*b"##p#");
const SPLASH_ART: FourCC = FourCC(*b"##s#");
const CAMPAIGN_END_ART: FourCC = FourCC(*b"inge");
const CAMPAIGN_END_TEXT: FourCC = FourCC(*b"epil");
const CAMPAIGN_END_SOUND: FourCC = FourCC(*b"naep");
const INGT: FourCC = FourCC(*b"ingt");
const DIFFICULTY_NAMES: FourCC = FourCC(*b"indl");
const DIFFICULTY_DESCRIPTIONS: FourCC = FourCC(*b"in04");
const DIFFICULTY_COUNT: usize = 5;
const BRIEFING_SEQUENCE: i32 = 3;
const MESH_NO_HINTS_DIALOG: u32 = 0x80_0000;

fn string_list(search: &TagSearch<'_>, tag: Option<FourCC>) -> Vec<Text> {
    tag.and_then(|tag| search.get(STRINGS, tag))
        .map(|data| string_list_lines(&data).into_iter().map(Text).collect())
        .unwrap_or_default()
}

fn text_tag(search: &TagSearch<'_>, tag: Option<FourCC>) -> Option<Text> {
    let data = search.get(TEXT, tag?)?;
    Some(Text(text_bytes(&data)))
}

fn read_difficulties(search: &TagSearch<'_>) -> Vec<Difficulty> {
    let descriptions = string_list(search, Some(DIFFICULTY_DESCRIPTIONS));
    string_list(search, Some(DIFFICULTY_NAMES))
        .into_iter()
        .take(DIFFICULTY_COUNT)
        .enumerate()
        .map(|(i, name)| Difficulty {
            name,
            description: descriptions.get(i).cloned().unwrap_or_default(),
        })
        .collect()
}

fn build_campaign(
    name: &[u8],
    checksum: i32,
    plugins: Vec<String>,
    entries: &[CachedEntry],
    search: &TagSearch<'_>,
    levels: &mut Vec<Level>,
) -> Campaign {
    let mut seen = HashSet::new();
    let mut to_read: Vec<FourCC> = entries.iter().rev().map(|e| e.mesh).collect();
    while let Some(mesh) = to_read.pop() {
        if !seen.insert(mesh) {
            continue;
        }
        if let Some(level) = read_level(search, mesh, checksum) {
            to_read.extend(level.alternate);
            to_read.extend(level.next);
            levels.push(level);
        }
    }
    Campaign {
        name: Text(name.to_vec()),
        checksum,
        plugins,
        entries: entries
            .iter()
            .map(|e| ListEntry {
                mesh: e.mesh,
                name: Text(e.name.clone()),
                flags: e.flags,
                order: e.order,
                extra_flags: e.extra_flags,
                plugin_checksum: checksum,
            })
            .collect(),
    }
}

fn read_level(search: &TagSearch<'_>, mesh: FourCC, plugin_checksum: i32) -> Option<Level> {
    let (links, file_type) = search.mesh_header(mesh)?;
    let art = |tag: Option<FourCC>| tag.filter(|&tag| search.has(COLLECTION, tag));
    let sound = |tag: Option<FourCC>| tag.filter(|&tag| search.has(SOUND, tag));
    let ambient = |tag: Option<FourCC>| tag.filter(|&tag| search.has(AMBIENT, tag));
    let link = |tag: Option<FourCC>| tag.filter(|&tag| search.mesh_header(tag).is_some());
    let strings = string_list(search, links.strings);
    Some(Level {
        mesh,
        plugin_checksum,
        title: strings.first().cloned().unwrap_or_default(),
        pre_art: art(links.pre_art).filter(|&tag| {
            search
                .sequence_count(tag)
                .is_some_and(|count| count > BRIEFING_SEQUENCE)
        }),
        briefing_text: text_tag(search, links.briefing_text),
        captions: string_list(search, links.captions),
        narration: sound(links.narration),
        post_art: art(links.post_art),
        next: link(links.next),
        alternate: link(links.alternate),
        ambient_win: ambient(links.ambient_win),
        ambient_loss: ambient(links.ambient_loss),
        flags: links.campaign_flags,
        file_type,
        objectives: join_objectives(&strings),
        hints: string_list(search, links.hints),
        no_hints_dialog: links.flags & MESH_NO_HINTS_DIALOG != 0,
        description: Text::default(),
        preview: art(Some(PREVIEW_ART)),
        splash: art(Some(SPLASH_ART)),
        cutscenes: search
            .mesh_header_bytes(mesh)
            .map(|header| {
                MeshTag::cutscene_names_from_bytes(&header)
                    .map(|name| name.map(|n| String::from_utf8_lossy(&n).into_owned()))
            })
            .unwrap_or_default(),
    })
}

fn join_objectives(strings: &[Text]) -> Text {
    let lines: Vec<&[u8]> = strings.iter().skip(1).map(|t| t.0.as_slice()).collect();
    Text(lines.join(&b'\r'))
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Campaign {
    pub name: Text,
    pub checksum: i32,
    pub plugins: Vec<String>,
    pub entries: Vec<ListEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListEntry {
    pub mesh: FourCC,
    pub name: Text,
    pub flags: u32,
    pub order: i16,
    pub extra_flags: u32,
    pub plugin_checksum: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Level {
    pub mesh: FourCC,
    pub plugin_checksum: i32,
    pub title: Text,
    pub pre_art: Option<FourCC>,
    pub briefing_text: Option<Text>,
    pub captions: Vec<Text>,
    pub narration: Option<FourCC>,
    pub post_art: Option<FourCC>,
    pub next: Option<FourCC>,
    pub alternate: Option<FourCC>,
    pub ambient_win: Option<FourCC>,
    pub ambient_loss: Option<FourCC>,
    pub flags: u32,
    pub file_type: i16,
    pub objectives: Text,
    pub hints: Vec<Text>,
    pub no_hints_dialog: bool,
    pub description: Text,
    pub preview: Option<FourCC>,
    pub splash: Option<FourCC>,
    pub cutscenes: [Option<String>; 3],
}

impl Level {
    pub fn new(mesh: FourCC) -> Self {
        Self {
            mesh,
            plugin_checksum: -1,
            title: Text::default(),
            pre_art: None,
            briefing_text: None,
            captions: Vec::new(),
            narration: None,
            post_art: None,
            next: None,
            alternate: None,
            ambient_win: None,
            ambient_loss: None,
            flags: 0,
            file_type: 0,
            objectives: Text::default(),
            hints: Vec::new(),
            no_hints_dialog: false,
            description: Text::default(),
            preview: None,
            splash: None,
            cutscenes: [None, None, None],
        }
    }
}
