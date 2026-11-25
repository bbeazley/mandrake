use crate::controls::{Diagnostic, line_of};
use mandrake_tag::tfl::Game;
use std::collections::BTreeMap;

pub(crate) fn number(item: &toml_edit::Item) -> Option<f64> {
    item.as_float()
        .or_else(|| item.as_integer().map(|v| v as f64))
}

pub(crate) enum Field<L> {
    Number {
        key: &'static str,
        slot: fn(&mut L) -> &mut Option<f32>,
        allowed: fn(f64) -> bool,
        range: &'static str,
    },
    Color {
        key: &'static str,
        slot: fn(&mut L) -> &mut Option<[f32; 3]>,
        example: &'static str,
        maps_only: bool,
    },
}

impl<L> Field<L> {
    fn key(&self) -> &'static str {
        match self {
            Field::Number { key, .. } | Field::Color { key, .. } => key,
        }
    }

    fn set(&self, layer: &mut L, item: &toml_edit::Item) -> Result<(), String> {
        match self {
            Field::Number {
                key,
                slot,
                allowed,
                range,
            } => {
                *slot(layer) = Some(match number(item) {
                    Some(v) if v.is_finite() && allowed(v) => v as f32,
                    Some(v) => return Err(format!("`{key}` is {v}, and must be {range}")),
                    None => return Err(format!("`{key}` must be a number, {range}")),
                });
            }
            Field::Color {
                key, slot, example, ..
            } => *slot(layer) = Some(color(item, key, example)?),
        }
        Ok(())
    }

    fn apply(&self, layer: &mut L, over: &mut L) {
        match self {
            Field::Number { slot, .. } => {
                if let Some(v) = *slot(over) {
                    *slot(layer) = Some(v);
                }
            }
            Field::Color { slot, .. } => {
                if let Some(v) = *slot(over) {
                    *slot(layer) = Some(v);
                }
            }
        }
    }

    fn is_set(&self, layer: &mut L) -> bool {
        match self {
            Field::Number { slot, .. } => slot(layer).is_some(),
            Field::Color { slot, .. } => slot(layer).is_some(),
        }
    }
}

fn color(item: &toml_edit::Item, key: &str, example: &str) -> Result<[f32; 3], String> {
    let parts: Option<Vec<f64>> = item.as_array().map(|a| {
        a.iter()
            .filter_map(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
            .collect()
    });
    match (parts.as_deref(), item.as_array().map(|a| a.len())) {
        (Some(&[r, g, b]), Some(3)) if [r, g, b].iter().all(|v| (0.0..=1.0).contains(v)) => {
            Ok([r as f32, g as f32, b as f32])
        }
        _ => Err(format!(
            "`{key}` must be three numbers from 0 to 1, as {example}"
        )),
    }
}

pub(crate) trait Layer: Default + Copy + 'static {
    const FIELDS: &'static [Field<Self>];
    const EXPECTED: [&'static str; 2];
    const LOWERCASE_TAGS: bool;

    fn apply(&mut self, over: &Self) {
        let mut over = *over;
        for field in Self::FIELDS {
            field.apply(self, &mut over);
        }
    }

    fn is_set(&self) -> bool {
        let mut layer = *self;
        Self::FIELDS.iter().any(|f| f.is_set(&mut layer))
    }
}

pub(crate) fn section(game: Game) -> &'static str {
    match game {
        Game::Myth2 => "map",
        Game::Tfl => "tfl.map",
    }
}

pub(crate) fn map_key(game: Game, mesh_tag: &str) -> String {
    match game {
        Game::Myth2 => mesh_tag.to_owned(),
        Game::Tfl => format!("tfl.{mesh_tag}"),
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct LayeredFile<L> {
    pub(crate) default: L,
    pub(crate) maps: BTreeMap<String, L>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Layered<L> {
    pub(crate) shipped: LayeredFile<L>,
    pub(crate) user: LayeredFile<L>,
}

impl<L: Layer> Layered<L> {
    pub(crate) fn of_shipped(shipped: &str, name: &str) -> Self {
        let (shipped, diags) = parse(shipped, "<defaults>");
        debug_assert!(
            diags.is_empty(),
            "shipped {name} file is invalid: {diags:?}"
        );
        Self {
            shipped,
            user: LayeredFile::default(),
        }
    }

    pub(crate) fn with_user(self, text: &str, source: &str) -> (Self, Vec<Diagnostic>) {
        let (user, diags) = parse(text, source);
        (Self { user, ..self }, diags)
    }

    fn key(game: Game, mesh_tag: &str) -> String {
        if L::LOWERCASE_TAGS {
            map_key(game, &mesh_tag.to_ascii_lowercase())
        } else {
            map_key(game, mesh_tag)
        }
    }

    pub fn has_map_in(&self, game: Game, mesh_tag: &str) -> bool {
        let key = Self::key(game, mesh_tag);
        self.shipped.maps.contains_key(&key) || self.user.maps.contains_key(&key)
    }

    pub(crate) fn layers(&self, game: Game, mesh_tag: &str) -> impl Iterator<Item = &L> {
        let key = Self::key(game, mesh_tag);
        [&self.shipped, &self.user]
            .into_iter()
            .flat_map(move |file| std::iter::once(&file.default).chain(file.maps.get(&key)))
    }

    pub(crate) fn resolve_layer_in(&self, game: Game, mesh_tag: &str, flags: &L) -> L {
        let mut layer = L::default();
        for l in self.layers(game, mesh_tag) {
            layer.apply(l);
        }
        layer.apply(flags);
        layer
    }
}

fn parse_layer<L: Layer>(
    table: &dyn toml_edit::TableLike,
    title: &str,
    line: &dyn Fn(&dyn toml_edit::TableLike, &str) -> Option<usize>,
    report: &mut dyn FnMut(Option<usize>, String),
) -> L {
    let mut layer = L::default();
    let is_map = title != "[default]";
    for (key, item) in table.iter() {
        let at = line(table, key);
        match L::FIELDS.iter().find(|f| f.key() == key) {
            Some(Field::Color {
                maps_only: true, ..
            }) if !is_map => report(
                at,
                format!("[default]: `{key}` is only allowed in a [map.<mesh tag>] table"),
            ),
            Some(field) => {
                if let Err(message) = field.set(&mut layer, item) {
                    report(at, format!("{title}: {message}"));
                }
            }
            None => report(
                at,
                format!(
                    "{title}: unknown key `{key}` (expected {})",
                    L::EXPECTED[usize::from(is_map)]
                ),
            ),
        }
    }
    layer
}

pub(crate) fn parse<L: Layer>(text: &str, source: &str) -> (LayeredFile<L>, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let mut file = LayeredFile::default();
    let mut report = |line: Option<usize>, message: String| {
        diags.push(Diagnostic {
            source: source.to_string(),
            line,
            message,
        })
    };
    let doc = match toml_edit::Document::parse(text.to_string()) {
        Ok(doc) => doc,
        Err(e) => {
            report(
                e.span().map(|r| line_of(text, r.start)),
                format!("TOML syntax error: {}", e.message().trim()),
            );
            return (file, diags);
        }
    };
    let line = |table: &dyn toml_edit::TableLike, key: &str| {
        table
            .get_key_value(key)
            .and_then(|(k, _)| k.span())
            .map(|r| line_of(text, r.start))
    };
    let root = doc.as_table();
    for (name, item) in root.iter() {
        let at = line(root, name);
        match (name, item.as_table_like()) {
            ("default", Some(table)) => {
                file.default = parse_layer(table, "[default]", &line, &mut report);
            }
            ("map", Some(maps)) => {
                parse_maps(maps, Game::Myth2, &mut file, &line, &mut report);
            }
            ("tfl", Some(tfl)) => {
                for (key, item) in tfl.iter() {
                    match (key, item.as_table_like()) {
                        ("map", Some(maps)) => {
                            parse_maps(maps, Game::Tfl, &mut file, &line, &mut report)
                        }
                        _ => report(
                            line(tfl, key),
                            format!("unknown key `tfl.{key}` (expected [tfl.map.<mesh tag>])"),
                        ),
                    }
                }
            }
            ("default" | "map" | "tfl", None) => report(at, format!("`{name}` must be a table")),
            _ => report(
                at,
                format!(
                    "unknown key `{name}` (expected [default], [map.<mesh tag>] or \
                     [tfl.map.<mesh tag>])"
                ),
            ),
        }
    }
    (file, diags)
}

fn parse_maps<L: Layer>(
    maps: &dyn toml_edit::TableLike,
    game: Game,
    file: &mut LayeredFile<L>,
    line: &dyn Fn(&dyn toml_edit::TableLike, &str) -> Option<usize>,
    report: &mut dyn FnMut(Option<usize>, String),
) {
    let section = section(game);
    for (tag, item) in maps.iter() {
        let at = line(maps, tag);
        let Some(table) = item.as_table_like() else {
            report(at, format!("`{section}.{tag}` must be a table"));
            continue;
        };
        if tag.len() != 4 {
            report(
                at,
                format!("[{section}.{tag}]: a mesh tag has four characters, as cr01"),
            );
            continue;
        }
        let layer = parse_layer(table, &format!("[{section}.{tag}]"), line, report);
        file.maps.insert(Layered::<L>::key(game, tag), layer);
    }
}
