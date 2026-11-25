use mandrake_common::{FourCC, M2Error};

use crate::collection::CollectionTag;
use crate::dialog::DialogDefinition;
use crate::font::FontTag;
use crate::header::decode_c_string;
use crate::interface::InterfaceDefinition;
use crate::sound::SoundTag;

fn expect_version(what: &str, id: FourCC, found: u16, expected: u16) -> Result<(), M2Error> {
    if found == expected {
        Ok(())
    } else {
        Err(M2Error::TagParse(format!(
            "TFL {what} '{id}': version {found}, the TFL 1.5 reader knows {expected}"
        )))
    }
}

pub const TFL_DIALOG_TAG_VERSION: u16 = 0;
pub const TFL_DIALOG_VERSION: u16 = 1;

pub const TFL_CONTROL_KINDS: [FourCC; 33] = [
    FourCC(*b"butt"),
    FourCC(*b"stat"),
    FourCC(*b"bitm"),
    FourCC(*b"radi"),
    FourCC(*b"chek"),
    FourCC(*b"edit"),
    FourCC(*b"list"),
    FourCC(*b"popu"),
    FourCC(*b"slst"),
    FourCC(*b"pass"),
    FourCC(*b"colr"),
    FourCC(*b"prog"),
    FourCC(*b"slid"),
    FourCC(*b"numb"),
    FourCC(*b"chat"),
    FourCC(*b"plli"),
    FourCC(*b"bann"),
    FourCC(*b"cgrf"),
    FourCC(*b"cstt"),
    FourCC(*b"ched"),
    FourCC(*b"room"),
    FourCC(*b"game"),
    FourCC(*b"map_"),
    FourCC(*b"popm"),
    FourCC(*b"gaml"),
    FourCC(*b"snap"),
    FourCC(*b"ipop"),
    FourCC(*b"STAT"),
    FourCC(*b"prol"),
    FourCC(*b"lstm"),
    FourCC(*b"keyb"),
    FourCC(*b"lstg"),
    FourCC(*b"repo"),
];

pub fn tfl_dialog(id: FourCC, tag_version: u16, d: &[u8]) -> Result<DialogDefinition, M2Error> {
    expect_version("bina header", id, tag_version, TFL_DIALOG_TAG_VERSION)?;
    let dialog =
        DialogDefinition::from_bytes(d).map_err(|e| M2Error::TagParse(format!("TFL {id}: {e}")))?;
    expect_version("bina", id, dialog.version, TFL_DIALOG_VERSION)?;
    let names_at =
        crate::dialog::DIALOG_HEADER_SIZE + crate::dialog::DIALOG_ITEM_SIZE * dialog.items.len();
    let names = d[names_at..].iter().filter(|&&b| b == 0).count();
    if names < dialog.items.len() {
        return Err(M2Error::TagParse(format!(
            "TFL bina '{id}': {names} names for {} items",
            dialog.items.len()
        )));
    }
    Ok(dialog)
}

pub fn unknown_controls(dialog: &DialogDefinition) -> Vec<FourCC> {
    dialog
        .items
        .iter()
        .map(|i| i.kind)
        .filter(|k| !TFL_CONTROL_KINDS.contains(k))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TflInterfaceDefinition {
    pub definition: InterfaceDefinition,
    pub unused_fonts: [Option<FourCC>; 2],
}

impl TflInterfaceDefinition {
    pub const SIZE: usize = 0x100;
    pub const VERSION: u16 = 1;

    pub fn from_tag(id: FourCC, version: u16, d: &[u8]) -> Result<Self, M2Error> {
        expect_version("inte", id, version, Self::VERSION)?;
        if d.len() != Self::SIZE {
            return Err(M2Error::TagParse(format!(
                "TFL inte '{id}': {} bytes, expected {}",
                d.len(),
                Self::SIZE
            )));
        }
        let r = |o: usize| {
            let b: [u8; 4] = d[o..o + 4].try_into().expect("in the tag");
            (b != [0xff; 4] && b != [0; 4]).then_some(FourCC(b))
        };
        Ok(Self {
            definition: InterfaceDefinition::from_bytes(d)?,
            unused_fonts: [r(0x1c), r(0x20)],
        })
    }
}

pub const TFL_STRING_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TflStringList {
    pub strings: Vec<Vec<u8>>,
}

impl TflStringList {
    pub fn from_tag(id: FourCC, version: u16, d: &[u8]) -> Result<Self, M2Error> {
        expect_version("stli", id, version, TFL_STRING_VERSION)?;
        Ok(Self {
            strings: d
                .split(|&b| b == 0 || b == b'\n' || b == b'\r')
                .filter(|s| !s.is_empty())
                .map(<[u8]>::to_vec)
                .collect(),
        })
    }

    pub fn get(&self, index: usize) -> &[u8] {
        self.strings.get(index).map_or(&[], Vec::as_slice)
    }

    pub fn text(&self, index: usize) -> String {
        decode_c_string(self.get(index))
    }

    pub fn len(&self) -> usize {
        self.strings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }
}

pub fn tfl_text(id: FourCC, version: u16, d: &[u8]) -> Result<Vec<u8>, M2Error> {
    expect_version("text", id, version, TFL_STRING_VERSION)?;
    let end = d.iter().position(|&b| b == 0).unwrap_or(d.len());
    Ok(d[..end].to_vec())
}

pub fn tfl_ditl(d: &[u8]) -> Vec<Vec<u8>> {
    let end = d.iter().position(|&b| b == 0).unwrap_or(d.len());
    d[..end]
        .split(|&b| b == b'\n' || b == b'\r')
        .map(<[u8]>::to_vec)
        .collect()
}

pub fn tfl_font(id: FourCC, version: u16, d: &[u8]) -> Result<FontTag, M2Error> {
    expect_version("font", id, version, TFL_STRING_VERSION)?;
    FontTag::from_bytes(d).map_err(|e| M2Error::TagParse(format!("TFL font '{id}': {e}")))
}

pub const TFL_COLLECTION_VERSION: u16 = 4;
pub const TFL_SOUND_VERSION: u16 = 1;

pub fn tfl_menu_collection(id: FourCC, version: u16, d: &[u8]) -> Result<CollectionTag, M2Error> {
    expect_version(".256", id, version, TFL_COLLECTION_VERSION)?;
    CollectionTag::from_bytes_tfl(d).map_err(|e| M2Error::TagParse(format!("TFL .256 '{id}': {e}")))
}

pub fn tfl_menu_sound(id: FourCC, version: u16, d: &[u8]) -> Result<SoundTag, M2Error> {
    expect_version("soun", id, version, TFL_SOUND_VERSION)?;
    SoundTag::from_bytes(d).map_err(|e| M2Error::TagParse(format!("TFL soun '{id}': {e}")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum TflGameState {
    Splash = 0,
    MainMenu = 1,
    Game = 2,
    EpilogueImage = 3,
    Quit = 4,
    Multiplayer = 7,
    Preferences = 9,
    NetworkGame = 10,
    Prologue = 11,
    Epilogue = 12,
    Intro = 13,
    QuitImage = 14,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TflMenuAction {
    LoadGame,
    Multiplayer,
    NewGame,
    Films,
    Tutorial,
    Intro,
    Preferences,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TflMainMenuButton {
    pub item: u32,
    pub art: u16,
    pub key: u8,
    pub action: TflMenuAction,
}

const fn button(item: u32, art: u16, key: u8, action: TflMenuAction) -> TflMainMenuButton {
    TflMainMenuButton {
        item,
        art,
        key,
        action,
    }
}

pub const TFL_MAIN_MENU: [TflMainMenuButton; 8] = [
    button(4, 1, b'C', TflMenuAction::LoadGame),
    button(3, 2, b'G', TflMenuAction::Multiplayer),
    button(2, 0, b'N', TflMenuAction::NewGame),
    button(5, 3, b'F', TflMenuAction::Films),
    button(6, 4, b'T', TflMenuAction::Tutorial),
    button(7, 5, b'I', TflMenuAction::Intro),
    button(8, 6, b'P', TflMenuAction::Preferences),
    button(9, 7, b'Q', TflMenuAction::Quit),
];

pub const TFL_MAIN_MENU_MUSIC: FourCC = FourCC(*b"scst");
pub const TFL_METASERVER_MUSIC: FourCC = FourCC(*b"scme");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TflScreen {
    pub name: &'static str,
    pub dialog: FourCC,
    pub collection: Option<FourCC>,
    pub strings: &'static [FourCC],
    pub sounds: &'static [FourCC],
}

macro_rules! screen {
    ($name:expr, $dialog:expr, $coll:expr, [$($s:expr),*], [$($snd:expr),*]) => {
        TflScreen {
            name: $name,
            dialog: FourCC(*$dialog),
            collection: $coll,
            strings: &[$(FourCC(*$s)),*],
            sounds: &[$(FourCC(*$snd)),*],
        }
    };
}

const fn some(id: &[u8; 4]) -> Option<FourCC> {
    Some(FourCC(*id))
}

pub const TFL_SCREENS: &[TflScreen] = &[
    screen!("splash", b"imdc", some(b"prsc"), [], []),
    screen!("main menu", b"main", some(b"main"), [], [b"scst"]),
    screen!("new game", b"nega", some(b"inng"), [b"indl"], []),
    screen!("prologue", b"prdi", None, [], []),
    screen!("epilogue", b"epdi", None, [], [b"scwi", b"sclo"]),
    screen!("saved games", b"opsg", some(b"in#1"), [], []),
    screen!("save game", b"sagd", some(b"insg"), [b"inrf"], []),
    screen!("saved films", b"serd", some(b"invs"), [b"inrf"], []),
    screen!("quit", b"qudi", some(b"ingp"), [], []),
    screen!("preferences", b"pref", some(b"inmp"), [], []),
    screen!(
        "keyboard preferences",
        b"kepd",
        some(b"incp"),
        [b"inan", b"inkn", b"inps", b"inmm"],
        []
    ),
    screen!("select protocol", b"nesp", some(b"inmg"), [], []),
    screen!("choose game", b"necg", None, [], []),
    screen!("TCP/IP", b"Inet", None, [], []),
    screen!("metaserver", b"medi", some(b"megr"), [b"inms"], [b"scme"]),
    screen!("configure game", b"ne#1", some(b"megr"), [b"inmn"], []),
    screen!("game options", b"nepd", some(b"ingo"), [], []),
    screen!("unit trading", b"untr", some(b"inut"), [], []),
    screen!("end of game", b"endi", some(b"megr"), [b"inmn"], []),
    screen!("postgame carnage report", b"pocr", None, [], []),
    screen!("progress", b"prdn", some(b"innb"), [], []),
    screen!("status bar", b"stba", some(b"baic"), [], []),
];

pub fn tfl_screen(dialog: FourCC) -> Option<&'static TflScreen> {
    TFL_SCREENS.iter().find(|s| s.dialog == dialog)
}

pub const TFL_INTRO_MOVIE: FourCC = FourCC(*b"00pr");
pub const TFL_FIRST_LEVEL: FourCC = FourCC(*b"ambu");
pub const TFL_TRAINING_LEVEL: FourCC = FourCC(*b"00tm");

#[cfg(test)]
mod tests {
    use super::*;

    fn fc(s: &[u8; 4]) -> FourCC {
        FourCC(*s)
    }

    #[test]
    fn strings_split_as_the_exe_does() {
        let s = TflStringList::from_tag(fc(b"indl"), 1, b"Timid\r\rSimple\r\nNormal\r").unwrap();
        assert_eq!(s.len(), 3);
        assert_eq!(s.get(1), b"Simple");
        assert_eq!(s.get(9), b"");
        assert!(TflStringList::from_tag(fc(b"indl"), 2, b"x").is_err());
        assert_eq!(
            tfl_ditl(b"OK\r\rCancel\r"),
            [b"OK".to_vec(), vec![], b"Cancel".to_vec(), vec![]]
        );
        assert_eq!(
            tfl_text(fc(b"01cb"), 1, b"one\rtwo\0x").unwrap(),
            b"one\rtwo"
        );
    }

    #[test]
    fn dialogs_check_versions_and_names() {
        let mut d = vec![0, 1, 0, 1, 0, 0, 0, 0, 1, 0xe0, 2, 0x80];
        d.extend_from_slice(b"butt");
        d.extend_from_slice(&[0, 0, 0, 2, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0]);
        d.extend_from_slice(&[0, 1, 0, 2, 0, 3, 0, 4]);
        assert!(tfl_dialog(fc(b"main"), 0, &d).is_err(), "no name");
        d.push(0);
        let dl = tfl_dialog(fc(b"main"), 0, &d).unwrap();
        assert_eq!(dl.items[0].id, 2);
        assert!(unknown_controls(&dl).is_empty());
        assert!(tfl_dialog(fc(b"main"), 1, &d).is_err());
        d[1] = 2;
        assert!(tfl_dialog(fc(b"main"), 0, &d).is_err());
    }

    #[test]
    fn the_interface_is_256_bytes() {
        let mut d = b"stinti1pco1bco#1go2bgo1bgo01".to_vec();
        d.resize(0x100, 0xff);
        let i = TflInterfaceDefinition::from_tag(fc(b"stin"), 1, &d).unwrap();
        assert_eq!(i.definition.font(5), Some(fc(b"go01")));
        assert_eq!(i.unused_fonts, [None, None]);
        assert!(TflInterfaceDefinition::from_tag(fc(b"stin"), 1, &d[..0x1c]).is_err());
        assert!(TflInterfaceDefinition::from_tag(fc(b"stin"), 2, &d).is_err());
    }
}
