use crate::text::Text;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PostgameReport {
    pub map: Text,
    pub result_word: Text,
    pub end_tick: i32,
    pub end_code: i16,
    pub winner: i16,
    pub film: bool,
    pub epilogue: Option<EpiloguePage>,
    pub pages: Option<FilmPages>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilmPages {
    pub game_type: i16,
    pub rank_columns: u8,
    pub rank: Vec<RankRow>,
    pub perm: Option<Vec<RankRow>>,
    pub graphs: Vec<Option<Graph>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RankRow {
    pub color: [u16; 3],
    pub team: bool,
    pub in_team: bool,
    pub cells: Vec<Text>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Graph {
    pub count: i32,
    pub period: i32,
    pub tick: i32,
    pub lines: Vec<GraphLine>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphLine {
    pub color: [u16; 3],
    pub values: Vec<i16>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EpiloguePage {
    pub title: Text,
    pub sections: Vec<ReportSection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportSection {
    pub kind: SectionKind,
    pub heading: Text,
    pub columns: Vec<Text>,
    pub rows: Vec<Vec<Text>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    Survivors,
    Heroes,
    Kills,
    DamageRatio,
    KillRatio { no_casualties: bool },
}
