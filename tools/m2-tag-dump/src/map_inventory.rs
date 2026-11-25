use mandrake_common::{FourCC, NativeVfs};
use mandrake_tag::TagArchive;
use mandrake_tag::map_action::{MapActionRecord, MeshMapActions};
use mandrake_tag::tag_files::TYPE_PLUGIN;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const PROCS_TSV: &str = include_str!("../data/map_action_procs.tsv");
const SIM_MAP_ACTIONS: &str = include_str!("../../../crates/mandrake-sim/src/map_actions.rs");

pub const REFERENCE_PLUGINS: [&str; 2] = ["Patch 1.8.5 Maps", "Patch 1.8.5 Unity"];

const MESH_FLAG_SOLO: u32 = 0x10000;

const PARTIAL: &[(&str, &[&str], &str)] = &[
    (
        "ctrl",
        &[],
        "parameters outside UNIT_CONTROL_PARAMETERS stop",
    ),
    ("tuni", &[], "parameters outside TEST_UNITS_PARAMETERS stop"),
    ("atta", &[], "proj naming a fade record's object"),
    (
        "geom",
        &[],
        "gameplay <= 0x6b8 branches; obje naming a fade record's object",
    ),
    (
        "lead",
        &[],
        "a #stt state from map data, a route index past the route",
    ),
    ("moef", &[], "material fades, projectiles, scale"),
];

#[derive(Debug, Clone)]
pub struct Proc {
    pub index: usize,
    pub tag: String,
    pub address: String,
    pub name: String,
    pub min_gameplay: String,
    pub flags: String,
}

pub fn procs() -> Vec<Proc> {
    let mut lines = PROCS_TSV.lines().filter(|l| !l.starts_with('#'));
    let header: Vec<&str> = lines.next().unwrap_or_default().split('\t').collect();
    let col = |name: &str| header.iter().position(|&h| h == name);
    let (index, tag, address, name, min_gameplay, flags) = (
        col("index"),
        col("tag"),
        col("proc"),
        col("name"),
        col("min_gameplay"),
        col("flags"),
    );
    lines
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            let get = |c: Option<usize>| c.and_then(|c| f.get(c)).map(|s| s.to_string());
            Some(Proc {
                index: get(index)?.parse().ok()?,
                tag: get(tag)?,
                address: get(address).unwrap_or_else(|| "-".to_string()),
                name: get(name)?,
                min_gameplay: get(min_gameplay)?,
                flags: get(flags)?,
            })
        })
        .collect()
}

fn byte_tags(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(k) = rest.find("b\"") {
        let s = &rest[k + 2..];
        match s.find('"') {
            Some(4) => {
                out.push(s[..4].to_string());
                rest = &s[5..];
            }
            Some(e) => rest = &s[e + 1..],
            None => break,
        }
    }
    out
}

pub fn ported_kinds(src: &str) -> BTreeSet<String> {
    let Some(start) = src.find("fn run_proc(") else {
        return BTreeSet::new();
    };
    let body = &src[start..];
    let end = body.find("_ => Err(Unmodelled").unwrap_or(body.len());
    body[..end]
        .lines()
        .filter(|l| l.trim_start().starts_with("b\"") && l.contains("=>"))
        .flat_map(|l| byte_tags(l.split("=>").next().unwrap_or("")))
        .collect()
}

pub fn const_list(src: &str, name: &str) -> Vec<String> {
    let Some(k) = src.find(&format!("const {name}: &[&[u8; 4]] = &[")) else {
        return Vec::new();
    };
    let body = &src[k..];
    let end = body.find("];").unwrap_or(body.len());
    byte_tags(&body[..end])
}

pub struct SimCoverage {
    pub ported: BTreeSet<String>,
    pub no_proc: bool,
    framework: Vec<String>,
    unit_control: Vec<String>,
    test_units: Vec<String>,
}

impl SimCoverage {
    pub fn from_source(src: &str) -> Self {
        Self {
            ported: ported_kinds(src),
            no_proc: src.contains("pub fn registered_map_action_proc("),
            framework: const_list(src, "FRAMEWORK_PARAMETERS"),
            unit_control: const_list(src, "UNIT_CONTROL_PARAMETERS"),
            test_units: const_list(src, "TEST_UNITS_PARAMETERS"),
        }
    }

    pub fn current() -> Self {
        Self::from_source(SIM_MAP_ACTIONS)
    }

    pub fn status(&self, kind: &str) -> &'static str {
        if !self.ported.contains(kind) {
            "no"
        } else if PARTIAL.iter().any(|(k, _, _)| *k == kind) {
            "partial"
        } else {
            "yes"
        }
    }

    pub fn unported_parameters(&self, kind: &str, params: &[String]) -> Vec<String> {
        let accepted = match kind {
            "ctrl" => Some(&self.unit_control),
            "tuni" => Some(&self.test_units),
            _ => None,
        };
        let mut out: Vec<String> = match accepted {
            Some(list) => params
                .iter()
                .filter(|p| !list.contains(p) && !self.framework.contains(p))
                .cloned()
                .collect(),
            None => {
                let names = PARTIAL
                    .iter()
                    .find(|(k, _, _)| *k == kind)
                    .map_or(&[][..], |(_, n, _)| n);
                params
                    .iter()
                    .filter(|p| names.contains(&p.as_str()))
                    .cloned()
                    .collect()
            }
        };
        out.dedup();
        out
    }
}

fn partial_note(kind: &str) -> &'static str {
    PARTIAL
        .iter()
        .find(|(k, _, _)| *k == kind)
        .map_or("", |(_, _, n)| n)
}

pub fn kind_of(a: &MapActionRecord) -> String {
    if a.tag == u32::MAX {
        "none".to_string()
    } else {
        FourCC::new(a.tag.to_be_bytes()).to_string()
    }
}

pub fn action_lines(acts: &MeshMapActions) -> Vec<String> {
    acts.actions
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let tag = FourCC::new(a.tag.to_be_bytes());
            let params: Vec<String> = a
                .parameters
                .iter()
                .map(|p| {
                    if p.kind == 1 {
                        let t: String = p
                            .values
                            .iter()
                            .take_while(|&&b| b != 0)
                            .map(|&b| b as u8 as char)
                            .collect();
                        format!("{}={t:?}", p.name)
                    } else if p.kind == 11 {
                        let t: Vec<String> = p
                            .values
                            .iter()
                            .map(|&v| FourCC::new((v as u32).to_be_bytes()).to_string())
                            .collect();
                        format!("{}=tags{t:?}", p.name)
                    } else if p.kind == 3 {
                        let t: Vec<String> = p
                            .values
                            .iter()
                            .map(|&v| format!("{:#06x}", v as u16))
                            .collect();
                        format!("{}=actions{t:?}", p.name)
                    } else {
                        format!("{}[{}]={:?}", p.name, p.kind, p.values)
                    }
                })
                .collect();
            format!(
                "{i}\t{:#06x}\t{}\t{tag}\t{:#x}\t{}+{}\t{}\t{}",
                a.identifier,
                a.kind,
                a.flags,
                a.delay,
                a.delay_range,
                a.time,
                params.join(" ")
            )
        })
        .collect()
}

pub struct InventoryMesh {
    pub mesh: String,
    pub source: String,
    pub group: &'static str,
    pub order: Option<i16>,
    pub name: String,
    pub actions: MeshMapActions,
}

fn c_text(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    b[..end]
        .iter()
        .map(|&c| match c {
            0xd0 => '\u{2013}',
            0xd1 => '\u{2014}',
            0xd2 => '\u{201c}',
            0xd3 => '\u{201d}',
            0xd4 => '\u{2018}',
            0xd5 => '\u{2019}',
            c => c as char,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn entry_info(a: &TagArchive, id: FourCC) -> Option<(i16, String)> {
    let e = a.entry_tags().iter().find(|e| e.tag_id == id)?;
    let file = c_text(&e.internal_name);
    let digits: String = file
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let mut name = e.display_name.to_vec();
    name.extend_from_slice(&e.description);
    Some((digits.parse().unwrap_or(0), c_text(&name)))
}

fn mesh_flags(data: &[u8]) -> u32 {
    data.get(0x4c..0x50)
        .map_or(0, |b| u32::from_be_bytes(b.try_into().unwrap()))
}

fn archive_label(path: &Path, a: &TagArchive) -> String {
    let name = a.header().name_as_str().trim().to_string();
    if name.is_empty() {
        path.file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    } else {
        name
    }
}

fn collect(
    archives: &[(PathBuf, TagArchive)],
    group_of: &dyn Fn(u32) -> &'static str,
) -> anyhow::Result<Vec<InventoryMesh>> {
    let mut ids: BTreeMap<FourCC, usize> = BTreeMap::new();
    for (k, (_, a)) in archives.iter().enumerate() {
        for e in a.find_tags_by_type(FourCC::new(*b"mesh")) {
            ids.insert(e.tag_id, k);
        }
    }
    let mut out = Vec::new();
    for (id, k) in ids {
        let (path, a) = &archives[k];
        let entry = a.find_tag_by_id(FourCC::new(*b"mesh"), id).unwrap();
        let data = a
            .get_tag_data(entry)
            .map_err(|e| anyhow::anyhow!("{}: mesh {id}: {e}", path.display()))?;
        let actions = MeshMapActions::from_mesh_bytes(&data)
            .map_err(|e| anyhow::anyhow!("{}: mesh {id}: {e}", path.display()))?;
        let info = archives.iter().rev().find_map(|(_, a)| entry_info(a, id));
        out.push(InventoryMesh {
            mesh: id.to_string(),
            source: archive_label(path, a),
            group: group_of(mesh_flags(&data)),
            order: info.as_ref().map(|i| i.0),
            name: info.map(|i| i.1).unwrap_or_default(),
            actions,
        });
    }
    Ok(out)
}

pub fn inventory(game_dir: &Path) -> anyhow::Result<Vec<InventoryMesh>> {
    let vfs = NativeVfs::default_cwd();
    let files = mandrake_tag::engine_tag_files(game_dir, &REFERENCE_PLUGINS);
    if !files.missing_plugins.is_empty() {
        anyhow::bail!("missing plugins: {:?}", files.missing_plugins);
    }
    let open = |p: &PathBuf| {
        TagArchive::open(&vfs, &p.to_string_lossy())
            .ok()
            .map(|a| (p.clone(), a))
    };
    let base: Vec<(PathBuf, TagArchive)> = files.paths.iter().filter_map(open).collect();
    let mut out = collect(&base, &|f| {
        if f & MESH_FLAG_SOLO != 0 {
            "campaign"
        } else {
            "base"
        }
    })?;
    let mut plugins: Vec<PathBuf> = std::fs::read_dir(game_dir.join("plugins"))
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    plugins.sort();
    for p in plugins
        .iter()
        .filter(|p| p.is_file() && !files.paths.contains(p))
    {
        let Some(a) = open(p) else { continue };
        if a.1.header().r#type != TYPE_PLUGIN {
            continue;
        }
        out.extend(collect(std::slice::from_ref(&a), &|_| "plugin")?);
    }
    Ok(out)
}

struct ActionRow {
    kind: String,
    params: Vec<String>,
    unported: Vec<String>,
}

pub fn effective_parameters(actions: &[MapActionRecord], action: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut stack = vec![action];
    let mut cur = 0;
    while cur < stack.len() {
        for p in &actions[stack[cur]].parameters {
            if p.kind == 3 && p.name.0 == *b"link" {
                for &id in p.values.iter().take(p.count.max(0) as usize) {
                    let id = id as u16;
                    let Some(ai) = (id != 0)
                        .then(|| actions.iter().position(|a| a.identifier == id))
                        .flatten()
                    else {
                        continue;
                    };
                    if (cur > 0 && stack[..cur].contains(&ai)) || stack.len() > 0x3f {
                        continue;
                    }
                    stack.push(ai);
                }
                continue;
            }
            let n = p.name.to_string();
            if !out.contains(&n) {
                out.push(n);
            }
        }
        cur += 1;
    }
    out
}

fn action_row(
    cov: &SimCoverage,
    table: &BTreeMap<String, Proc>,
    actions: &[MapActionRecord],
    i: usize,
) -> ActionRow {
    let a = &actions[i];
    let kind = kind_of(a);
    let mut params: Vec<String> = a.parameters.iter().map(|p| p.name.to_string()).collect();
    params.dedup();
    let mut unported = Vec::new();
    if kind != "none" {
        if !table.contains_key(&kind) {
            if !cov.no_proc {
                unported.push("not in proc table".to_string());
            }
        } else if !cov.ported.contains(&kind) {
            unported.push("proc".to_string());
        } else {
            unported.extend(cov.unported_parameters(&kind, &effective_parameters(actions, i)));
        }
    }
    ActionRow {
        kind,
        params,
        unported,
    }
}

fn sort_key(m: &InventoryMesh) -> (u8, String, i16, String) {
    let g = match m.group {
        "campaign" => 0,
        "base" => 1,
        _ => 2,
    };
    let src = if g == 2 {
        m.source.clone()
    } else {
        String::new()
    };
    (g, src, m.order.unwrap_or(i16::MAX), m.mesh.clone())
}

fn counts(map: &BTreeMap<String, usize>) -> String {
    let mut v: Vec<(&String, &usize)> = map.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let s: Vec<String> = v.iter().map(|(k, n)| format!("{k}:{n}")).collect();
    if s.is_empty() {
        "-".into()
    } else {
        s.join(",")
    }
}

fn or_dash(v: &[String]) -> String {
    if v.is_empty() {
        "-".into()
    } else {
        v.join(",")
    }
}

pub fn tables(meshes: &[InventoryMesh], cov: &SimCoverage) -> [(&'static str, String); 3] {
    let table: BTreeMap<String, Proc> = procs().into_iter().map(|p| (p.tag.clone(), p)).collect();
    let mut sorted: Vec<&InventoryMesh> = meshes.iter().collect();
    sorted.sort_by_key(|m| sort_key(m));
    let regen = "# Generated by m2-tag-dump map-actions-inventory; do not edit.\n";

    #[derive(Default)]
    struct KindStats {
        meshes: BTreeSet<(String, String)>,
        campaign: BTreeSet<String>,
        actions: usize,
        params: BTreeMap<String, usize>,
        flags: BTreeMap<String, usize>,
        unported: BTreeMap<String, usize>,
    }
    let mut kinds: BTreeMap<String, KindStats> = table
        .keys()
        .map(|k| (k.clone(), KindStats::default()))
        .collect();

    let mut by_mesh = String::from(regen);
    by_mesh.push_str(
        "# proc = index in registered_map_action_procs 0x6608c0 (static order, map_action_procs.tsv); \
         unported = proc (the sim has none), a parameter the sim's proc doesn't port, or -.\n",
    );
    by_mesh.push_str("mesh\tsource\tindex\tidentifier\tkind\tproc\tflags\tparameters\tunported\n");
    let mut summary = String::from(regen);
    summary.push_str(
        "# group: campaign = solo mesh (+0x4c & 0x10000) of the 1.8.5 set; base = the set's others; \
         plugin = an installed plugin's.\n# not_ported = kinds the sim has no proc for; \
         partial_hits = kind:parameter the sim's proc doesn't port.\n",
    );
    summary
        .push_str("mesh\tsource\tgroup\torder\tname\tactions\tkinds\tnot_ported\tpartial_hits\n");
    for m in &sorted {
        let mut mesh_kinds: BTreeSet<String> = BTreeSet::new();
        let mut not_ported: BTreeSet<String> = BTreeSet::new();
        let mut partial: BTreeSet<String> = BTreeSet::new();
        for (i, a) in m.actions.actions.iter().enumerate() {
            let r = action_row(cov, &table, &m.actions.actions, i);
            let proc = table
                .get(&r.kind)
                .map_or("-".to_string(), |p| p.index.to_string());
            let _ = writeln!(
                by_mesh,
                "{}\t{}\t{i}\t{:#06x}\t{}\t{proc}\t{:#x}\t{}\t{}",
                m.mesh,
                m.source,
                a.identifier,
                r.kind,
                a.flags,
                or_dash(&r.params),
                or_dash(&r.unported)
            );
            if r.kind == "none" {
                continue;
            }
            mesh_kinds.insert(r.kind.clone());
            let s = kinds.entry(r.kind.clone()).or_default();
            s.meshes.insert((m.source.clone(), m.mesh.clone()));
            if m.group == "campaign" {
                s.campaign.insert(m.mesh.clone());
            }
            s.actions += 1;
            for p in &r.params {
                *s.params.entry(p.clone()).or_default() += 1;
            }
            *s.flags.entry(format!("{:#x}", a.flags)).or_default() += 1;
            if r.unported
                .first()
                .is_some_and(|u| u == "proc" || u == "not in proc table")
            {
                not_ported.insert(r.kind.clone());
            } else {
                for u in &r.unported {
                    *s.unported.entry(u.clone()).or_default() += 1;
                    partial.insert(format!("{}:{u}", r.kind));
                }
            }
        }
        let _ = writeln!(
            summary,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            m.mesh,
            m.source,
            m.group,
            m.order.map_or("-".to_string(), |o| o.to_string()),
            if m.name.is_empty() { "-" } else { &m.name },
            m.actions.actions.len(),
            mesh_kinds.len(),
            or_dash(&not_ported.into_iter().collect::<Vec<_>>()),
            or_dash(&partial.into_iter().collect::<Vec<_>>())
        );
    }

    let mut by_kind = String::from(regen);
    by_kind.push_str(
        "# ported: yes / partial (the sim's proc names branches it doesn't port: note) / no (no proc in \
         run_proc, crates/mandrake-sim/src/map_actions.rs). meshes and actions count every mesh row; \
         campaign = solo meshes of the 1.8.5 set.\n# parameters and flags: name:count over the \
         actions; unported_seen: parameters the sim's proc doesn't port, as used in the data.\n",
    );
    by_kind.push_str(
        "kind\tproc\taddress\tname\tmin_gameplay\tlocal\tported\tmeshes\tactions\tcampaign_meshes\tparameters\tflags\tunported_seen\tnote\n",
    );
    let mut order: Vec<(&String, &KindStats)> = kinds.iter().collect();
    order.sort_by(|a, b| {
        b.1.meshes
            .len()
            .cmp(&a.1.meshes.len())
            .then(b.1.actions.cmp(&a.1.actions))
            .then(a.0.cmp(b.0))
    });
    for (k, s) in order {
        let p = table.get(k);
        let status = if p.is_none() {
            if cov.no_proc {
                "yes (not in proc table: the trigger fails, result 1)"
            } else {
                "no (not in proc table)"
            }
        } else {
            cov.status(k)
        };
        let _ = writeln!(
            by_kind,
            "{k}\t{}\t{}\t{}\t{}\t{}\t{status}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            p.map_or("-".to_string(), |p| p.index.to_string()),
            p.map_or("-", |p| p.address.as_str()),
            p.map_or("-", |p| p.name.as_str()),
            p.map_or("-", |p| p.min_gameplay.as_str()),
            p.map_or("-", |p| if p.flags == "1" { "yes" } else { "no" }),
            s.meshes.len(),
            s.actions,
            s.campaign.len(),
            counts(&s.params),
            counts(&s.flags),
            counts(&s.unported),
            if partial_note(k).is_empty() {
                "-"
            } else {
                partial_note(k)
            },
        );
    }
    [
        ("by-mesh.tsv", by_mesh),
        ("by-kind.tsv", by_kind),
        ("by-mesh-summary.tsv", summary),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sim_coverage_reads_the_dispatch() {
        let cov = SimCoverage::current();
        for k in ["soun", "ctrl", "tuni", "plmo", "plsc", "anim", "endg"] {
            assert!(cov.ported.contains(k), "{k} should be ported");
        }
        assert!(!cov.ported.contains("math"));
        assert!(cov.no_proc);
        let table: BTreeSet<String> = procs().into_iter().map(|p| p.tag).collect();
        assert_eq!(table.len(), 62);
        for k in &cov.ported {
            assert!(table.contains(k), "{k} is not in the proc table");
        }
        for (k, _, _) in PARTIAL {
            assert!(cov.ported.contains(*k), "partial kind {k} isn't ported");
        }
        assert!(cov.unit_control.contains(&"hefr".to_string()));
        assert!(cov.unit_control.contains(&"heig".to_string()));
        assert!(cov.test_units.contains(&"hei>".to_string()));
        assert!(cov.framework.contains(&"trsi".to_string()));
        let p = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            cov.unported_parameters("ctrl", &p(&["subj", "held", "acoa"])),
            p(&["held"])
        );
        assert!(
            cov.unported_parameters("atta", &p(&["subj", "chrm"]))
                .is_empty()
        );
        assert!(cov.unported_parameters("soun", &p(&["soun"])).is_empty());
    }
}
