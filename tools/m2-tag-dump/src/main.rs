use clap::{Parser, Subcommand};
use mandrake_common::{FourCC, NativeVfs};
use mandrake_tag::interface::{
    COLOR_SELECTION_ALLY, COLOR_SELECTION_ENEMY, COLOR_SELECTION_OBJECT, COLOR_SELECTION_OWNED,
    COLOR_VITALITY_BACKGROUND_BASE, COLOR_VITALITY_FILL_BASE, STANDARD_INTERFACE_COLLECTION,
    VITALITY_COLOR_STEPS,
};
use mandrake_tag::{
    CollectionTag, InterfaceColorTable, MeshTag, ObjectDefinition, ProjectileDefinition,
    ProjectileGroupDefinition, StringListTag, TagArchive, TagEntry, UnitTag, displayed_views,
};
use std::fs;
use std::path::Path;

mod colperm_report;
mod combanim_report;
mod detail_report;
mod hud_report;
mod hud_text;
mod lighting_report;
mod map_inventory;
mod media_report;
mod movemark_report;
mod projfx_report;
mod unitanim_report;

#[derive(Parser, Debug)]
#[command(
    name = "m2-tag-dump",
    about = "Inspect and dump Myth II tag archives and deserialized tag contents",
    version
)]
struct Args {
    #[arg(
        help = "Path to the monolithic tag archive (e.g. 'small install', 'large install')",
        default_value = ""
    )]
    archive: String,

    #[arg(
        short = 't',
        long = "type",
        help = "Filter tags by FourCC type (e.g. 'mesh', 'stli', 'coll', 'unit')"
    )]
    filter_type: Option<String>,

    #[arg(
        short = 'n',
        long = "name",
        help = "Filter tags by name (case-insensitive substring)"
    )]
    filter_name: Option<String>,

    #[arg(
        short = 'l',
        long = "limit",
        help = "Limit the number of tags displayed"
    )]
    limit: Option<usize>,

    #[arg(
        short = 'i',
        long = "info",
        help = "Display only header metadata and entry tags"
    )]
    info_only: bool,

    #[arg(
        short = 'e',
        long = "extract",
        help = "Extract raw tag payload by FourCC ID"
    )]
    extract_id: Option<String>,

    #[arg(
        short = 'o',
        long = "output",
        help = "Output file path when extracting tag payload"
    )]
    output: Option<String>,

    #[arg(
        long = "inspect-mesh",
        help = "Inspect detailed mesh tag by ID or name"
    )]
    inspect_mesh: Option<String>,

    #[arg(
        long = "inspect-strings",
        help = "Inspect detailed string list tag by ID or name"
    )]
    inspect_strings: Option<String>,

    #[arg(
        long = "inspect-coll",
        help = "Inspect detailed collection tag by ID or name"
    )]
    inspect_coll: Option<String>,

    #[arg(
        long = "inspect-unit",
        help = "Inspect detailed unit/monster tag by ID or name"
    )]
    inspect_unit: Option<String>,

    #[arg(
        long = "inspect-selection",
        help = "Inspect the selection frame / vitality bar inputs of a monster (mons ID or name): \
                its obje definition, every sequence's selection rectangle and the 'stin' \
                interface colors (use --coll-archive for the animation collection)"
    )]
    inspect_selection: Option<String>,

    #[arg(
        long = "inspect-proj",
        help = "Inspect a projectile (proj) definition by ID or name: flags, tag references, \
                sounds and the damage definition at +0x68"
    )]
    inspect_proj: Option<String>,

    #[arg(
        long = "inspect-prgr",
        help = "Inspect a projectile group (prgr) definition by ID or name: sound and parts"
    )]
    inspect_prgr: Option<String>,

    #[arg(
        long = "coll-archive",
        help = "Archive holding the unit's animation collection (e.g. 'large install'); resolves sequence slot names for --inspect-unit"
    )]
    coll_archive: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
#[allow(clippy::enum_variant_names)]
enum Commands {
    InspectMesh {
        archive: String,
        mesh_id: String,
    },
    InspectStrings {
        archive: String,
        tag_id: String,
    },
    InspectColl {
        archive: String,
        coll_id: String,
    },
    InspectUnit {
        archive: String,
        unit_id: String,
        #[arg(long)]
        coll_archive: Option<String>,
    },
    ExportObj {
        archive: String,
        mesh_id: String,
        output: String,
        #[arg(long, default_value_t = 0.001953125)]
        height_scale: f32,
    },
    DumpRaw {
        archive: String,
        tag_type: String,
        tag_id: String,
        output: String,
    },
    MediaReport {
        archives: Vec<String>,
    },
    MediaLod {
        archive: String,
        mesh_id: String,
        #[arg(long, value_delimiter = ',')]
        region: Option<Vec<usize>>,
    },
    MediaObjects {
        archive: String,
        mesh_id: String,
    },
    LightingReport {
        archives: Vec<String>,
    },
    ColpermReport {
        game_dir: String,
        meshes: Vec<String>,
    },
    CombanimReport {
        game_dir: String,
        meshes: Vec<String>,
        #[arg(long, value_delimiter = ',')]
        mons: Vec<String>,
    },
    ProjfxReport {
        game_dir: String,
        meshes: Vec<String>,
        #[arg(long, value_delimiter = ',')]
        medi: Vec<String>,
        #[arg(long, value_delimiter = ',')]
        proj: Vec<String>,
    },
    MovemarkReport {
        game_dir: String,
        meshes: Vec<String>,
        #[arg(long, value_delimiter = ',', default_value = "stin")]
        inte: Vec<String>,
        #[arg(long, allow_hyphen_values = true)]
        point: Vec<String>,
        #[arg(long, default_value_t = 4096)]
        radius: i32,
        #[arg(long)]
        grid: bool,
    },
    UnitanimReport {
        game_dir: String,
        mesh: String,
        #[arg(long)]
        film: Option<String>,
        #[arg(long, default_value_t = 0)]
        seed: u32,
        #[arg(long, default_value_t = 100)]
        ticks: u32,
        #[arg(long = "move")]
        moves: Vec<String>,
        #[arg(long, value_delimiter = ',')]
        ids: Vec<u32>,
    },
    HudReport {
        game_dir: String,
        meshes: Vec<String>,
        #[arg(long, default_values_t = vec!["1024x768".to_string(), "3840x2160".to_string()])]
        size: Vec<String>,
    },
    MeshFlags {
        game_dir: String,
    },
    MapActions {
        game_dir: String,
        mesh: String,
        #[arg(long)]
        plugin: Vec<String>,
    },
    MapActionsInventory {
        game_dir: String,
        #[arg(long)]
        out: String,
    },
    DetailMap {
        game_dir: String,
        mesh: String,
        dmap_archive: String,
        #[arg(long)]
        plugin: Vec<String>,
    },
    HudTitle {
        game_dir: String,
        reference: String,
        #[arg(long, default_value = "Tutorial")]
        scoring: String,
        #[arg(long, default_value_t = 0)]
        difficulty: usize,
        #[arg(long)]
        out: Option<String>,
        #[arg(long, default_value_t = 8)]
        zoom: u32,
    },
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    if let Some(cmd) = args.command {
        match cmd {
            Commands::InspectMesh { archive, mesh_id } => {
                let arc = open_archive(&archive)?;
                return inspect_mesh(&arc, &mesh_id);
            }
            Commands::InspectStrings { archive, tag_id } => {
                let arc = open_archive(&archive)?;
                return inspect_strings(&arc, &tag_id);
            }
            Commands::InspectColl { archive, coll_id } => {
                let arc = open_archive(&archive)?;
                return inspect_coll(&arc, &coll_id);
            }
            Commands::InspectUnit {
                archive,
                unit_id,
                coll_archive,
            } => {
                let arc = open_archive(&archive)?;
                let coll_arc = coll_archive.as_deref().map(open_archive).transpose()?;
                return inspect_unit(&arc, &unit_id, coll_arc.as_ref());
            }
            Commands::ExportObj {
                archive,
                mesh_id,
                output,
                height_scale,
            } => {
                let arc = open_archive(&archive)?;
                return export_obj(&arc, &mesh_id, &output, height_scale);
            }
            Commands::DumpRaw {
                archive,
                tag_type,
                tag_id,
                output,
            } => {
                let arc = open_archive(&archive)?;
                let entry = find_tag_any(&arc, &tag_type, &tag_id)
                    .ok_or_else(|| anyhow::anyhow!("No {} tag '{}'", tag_type, tag_id))?;
                let data = arc
                    .get_tag_data(entry)
                    .map_err(|e| anyhow::anyhow!("Failed to read tag: {}", e))?;
                std::fs::write(&output, &data)?;
                println!(
                    "{} '{}' ({}): {} bytes -> {}",
                    entry.tag_type,
                    entry.tag_id,
                    entry.name,
                    data.len(),
                    output
                );
                return Ok(());
            }
            Commands::MediaReport { archives } => {
                let arcs = archives
                    .iter()
                    .map(|a| {
                        Ok((
                            a.rsplit('/').next().unwrap_or(a).to_string(),
                            open_archive(a)?,
                        ))
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;
                return media_report::media_report(&arcs);
            }
            Commands::MediaLod {
                archive,
                mesh_id,
                region,
            } => {
                let arc = open_archive(&archive)?;
                let entry = find_tag_any(&arc, "mesh", &mesh_id)
                    .ok_or_else(|| anyhow::anyhow!("No mesh '{}'", mesh_id))?;
                let region = match region.as_deref() {
                    Some([a, b, c, d]) => Some([*a, *b, *c, *d]),
                    Some(_) => anyhow::bail!("--region needs X0,Y0,X1,Y1"),
                    None => None,
                };
                return media_report::media_lod(&arc, entry, region);
            }
            Commands::LightingReport { archives } => {
                let arcs = archives
                    .iter()
                    .map(|a| {
                        Ok((
                            a.rsplit('/').next().unwrap_or(a).to_string(),
                            open_archive(a)?,
                        ))
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;
                return lighting_report::lighting_report(&arcs);
            }
            Commands::ColpermReport { game_dir, meshes } => {
                let files = mandrake_tag::engine_tag_files(Path::new(&game_dir), &[]);
                let vfs = NativeVfs::default_cwd();
                let arcs: Vec<TagArchive> = files
                    .paths
                    .iter()
                    .filter_map(|p| TagArchive::open(&vfs, &p.to_string_lossy()).ok())
                    .collect();
                println!("# tag files (later override earlier):");
                for p in &files.paths {
                    println!("#   {}", p.display());
                }
                let tags = |group: [u8; 4], id: FourCC| {
                    arcs.iter().rev().find_map(|a| {
                        a.find_tag_by_id(FourCC::new(group), id)
                            .and_then(|e| a.get_tag_data(e).ok())
                    })
                };
                for m in &meshes {
                    let id = FourCC::try_from(m.as_str())
                        .map_err(|e| anyhow::anyhow!("mesh id '{m}': {e:?}"))?;
                    colperm_report::colperm_report(id, &tags)?;
                    println!();
                }
                return Ok(());
            }
            Commands::CombanimReport {
                game_dir,
                meshes,
                mons,
            } => {
                let files = mandrake_tag::engine_tag_files(Path::new(&game_dir), &[]);
                let vfs = NativeVfs::default_cwd();
                let arcs: Vec<TagArchive> = files
                    .paths
                    .iter()
                    .filter_map(|p| TagArchive::open(&vfs, &p.to_string_lossy()).ok())
                    .collect();
                println!("# tag files (later override earlier):");
                for p in &files.paths {
                    println!("#   {}", p.display());
                }
                let tags = |group: [u8; 4], id: FourCC| {
                    arcs.iter().rev().find_map(|a| {
                        a.find_tag_by_id(FourCC::new(group), id)
                            .and_then(|e| a.get_tag_data(e).ok())
                    })
                };
                for m in &meshes {
                    let id = FourCC::try_from(m.as_str())
                        .map_err(|e| anyhow::anyhow!("mesh id '{m}': {e:?}"))?;
                    combanim_report::combanim_report(id, &tags)?;
                    println!();
                }
                for m in &mons {
                    let id = FourCC::try_from(m.as_str())
                        .map_err(|e| anyhow::anyhow!("mons id '{m}': {e:?}"))?;
                    println!("\n## extra mons {id}");
                    combanim_report::report_mons(&tags, id)?;
                }
                return Ok(());
            }
            Commands::ProjfxReport {
                game_dir,
                meshes,
                medi,
                proj,
            } => {
                let files = mandrake_tag::engine_tag_files(Path::new(&game_dir), &[]);
                let vfs = NativeVfs::default_cwd();
                let arcs: Vec<TagArchive> = files
                    .paths
                    .iter()
                    .filter_map(|p| TagArchive::open(&vfs, &p.to_string_lossy()).ok())
                    .collect();
                println!("# tag files (later override earlier):");
                for p in &files.paths {
                    println!("#   {}", p.display());
                }
                let tags = |group: [u8; 4], id: FourCC| {
                    arcs.iter().rev().find_map(|a| {
                        a.find_tag_by_id(FourCC::new(group), id)
                            .and_then(|e| a.get_tag_data(e).ok())
                    })
                };
                let ids = |v: &[String]| -> anyhow::Result<Vec<FourCC>> {
                    v.iter()
                        .map(|m| {
                            FourCC::try_from(m.as_str())
                                .map_err(|e| anyhow::anyhow!("tag id '{m}': {e:?}"))
                        })
                        .collect()
                };
                projfx_report::projfx_report(&tags, &ids(&meshes)?, &ids(&medi)?, &ids(&proj)?)?;
                return Ok(());
            }
            Commands::HudTitle {
                game_dir,
                reference,
                scoring,
                difficulty,
                out,
                zoom,
            } => {
                let files = mandrake_tag::engine_interface_tag_files(Path::new(&game_dir), &[]);
                let tags = mandrake_tag::TagLookup::open(&files);
                hud_text::hud_title(
                    &tags,
                    Path::new(&reference),
                    &scoring,
                    difficulty,
                    out.as_deref().map(Path::new),
                    zoom,
                )?;
                return Ok(());
            }
            Commands::MapActions {
                game_dir,
                mesh,
                plugin,
            } => {
                let plugins: Vec<&str> = plugin.iter().map(String::as_str).collect();
                let files = mandrake_tag::engine_tag_files(Path::new(&game_dir), &plugins);
                let vfs = NativeVfs::default_cwd();
                let id = FourCC::try_from(mesh.as_str())
                    .map_err(|e| anyhow::anyhow!("tag id '{mesh}': {e:?}"))?;
                let data = files
                    .paths
                    .iter()
                    .rev()
                    .filter_map(|p| TagArchive::open(&vfs, &p.to_string_lossy()).ok())
                    .find_map(|a| {
                        a.find_tag_by_id(FourCC::new(*b"mesh"), id)
                            .and_then(|e| a.get_tag_data(e).ok())
                    })
                    .ok_or_else(|| anyhow::anyhow!("mesh {mesh} not found"))?;
                let acts = mandrake_tag::map_action::MeshMapActions::from_mesh_bytes(&data)
                    .map_err(|e| anyhow::anyhow!(e))?;
                println!(
                    "# mesh {mesh}: {} map actions (index identifier kind tag flags delay+range time: parameters)",
                    acts.actions.len()
                );
                for line in map_inventory::action_lines(&acts) {
                    println!("{line}");
                }
                return Ok(());
            }
            Commands::MapActionsInventory { game_dir, out } => {
                let meshes = map_inventory::inventory(Path::new(&game_dir))?;
                let cov = map_inventory::SimCoverage::current();
                fs::create_dir_all(&out)?;
                for (name, text) in map_inventory::tables(&meshes, &cov) {
                    let path = Path::new(&out).join(name);
                    fs::write(&path, text)?;
                    println!("{}", path.display());
                }
                println!("{} meshes", meshes.len());
                return Ok(());
            }
            Commands::DetailMap {
                game_dir,
                mesh,
                dmap_archive,
                plugin,
            } => {
                let plugins: Vec<&str> = plugin.iter().map(String::as_str).collect();
                let files = mandrake_tag::engine_tag_files(Path::new(&game_dir), &plugins);
                let vfs = NativeVfs::default_cwd();
                let id = FourCC::try_from(mesh.as_str())
                    .map_err(|e| anyhow::anyhow!("tag id '{mesh}': {e:?}"))?;
                let data = files
                    .paths
                    .iter()
                    .rev()
                    .filter_map(|p| TagArchive::open(&vfs, &p.to_string_lossy()).ok())
                    .find_map(|a| {
                        a.find_tag_by_id(FourCC::new(*b"mesh"), id)
                            .and_then(|e| a.get_tag_data(e).ok())
                    })
                    .ok_or_else(|| anyhow::anyhow!("mesh {mesh} not found"))?;
                return detail_report::run(&mesh, &data, &open_archive(&dmap_archive)?);
            }
            Commands::MeshFlags { game_dir } => {
                let root = Path::new(&game_dir);
                let mut paths = mandrake_tag::engine_tag_files(root, &[]).paths;
                if let Ok(rd) = fs::read_dir(root.join("plugins")) {
                    let mut plugins: Vec<_> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
                    plugins.sort();
                    paths.extend(plugins.into_iter().filter(|p| p.is_file()));
                }
                let vfs = NativeVfs::default_cwd();
                let arcs: Vec<(String, TagArchive)> = paths
                    .iter()
                    .filter_map(|p| {
                        let name = p.file_name()?.to_string_lossy().into_owned();
                        TagArchive::open(&vfs, &p.to_string_lossy())
                            .ok()
                            .map(|a| (name, a))
                    })
                    .collect();
                hud_report::mesh_flags_report(&arcs)?;
                return Ok(());
            }
            Commands::UnitanimReport {
                game_dir,
                mesh,
                film,
                seed,
                ticks,
                moves,
                ids,
            } => {
                let moves = moves
                    .iter()
                    .map(|m| unitanim_report::parse_move(m))
                    .collect::<anyhow::Result<Vec<_>>>()?;
                print!(
                    "{}",
                    unitanim_report::report(
                        Path::new(&game_dir),
                        &mesh,
                        film.as_deref().map(Path::new),
                        seed,
                        ticks,
                        &moves,
                        &ids,
                    )?
                );
                return Ok(());
            }
            Commands::HudReport {
                game_dir,
                meshes,
                size,
            } => {
                let mut files = mandrake_tag::engine_tag_files(Path::new(&game_dir), &[]);
                for n in ["international large install", "international small install"] {
                    let p = Path::new(&game_dir).join("tags").join(n);
                    if p.is_file() {
                        files.paths.insert(0, p);
                    }
                }
                let vfs = NativeVfs::default_cwd();
                let arcs: Vec<TagArchive> = files
                    .paths
                    .iter()
                    .filter_map(|p| TagArchive::open(&vfs, &p.to_string_lossy()).ok())
                    .collect();
                println!("# tag files (later override earlier):");
                for p in &files.paths {
                    println!("#   {}", p.display());
                }
                let tags = |group: [u8; 4], id: FourCC| {
                    arcs.iter().rev().find_map(|a| {
                        a.find_tag_by_id(FourCC::new(group), id)
                            .and_then(|e| a.get_tag_data(e).ok())
                    })
                };
                let meshes = meshes
                    .iter()
                    .map(|m| {
                        FourCC::try_from(m.as_str())
                            .map_err(|e| anyhow::anyhow!("tag id '{m}': {e:?}"))
                    })
                    .collect::<anyhow::Result<Vec<FourCC>>>()?;
                let sizes = size
                    .iter()
                    .map(|s| {
                        let (w, h) = s
                            .split_once('x')
                            .ok_or_else(|| anyhow::anyhow!("size '{s}': want WIDTHxHEIGHT"))?;
                        Ok((w.trim().parse()?, h.trim().parse()?))
                    })
                    .collect::<anyhow::Result<Vec<(i32, i32)>>>()?;
                hud_report::hud_report(&tags, &meshes, &sizes)?;
                return Ok(());
            }
            Commands::MovemarkReport {
                game_dir,
                meshes,
                inte,
                point,
                radius,
                grid,
            } => {
                let mut files = mandrake_tag::engine_tag_files(Path::new(&game_dir), &[]);
                for n in ["international large install", "international small install"] {
                    let p = Path::new(&game_dir).join("tags").join(n);
                    if p.is_file() {
                        files.paths.insert(0, p);
                    }
                }
                let vfs = NativeVfs::default_cwd();
                let arcs: Vec<TagArchive> = files
                    .paths
                    .iter()
                    .filter_map(|p| TagArchive::open(&vfs, &p.to_string_lossy()).ok())
                    .collect();
                println!("# tag files (later override earlier):");
                for p in &files.paths {
                    println!("#   {}", p.display());
                }
                let tags = |group: [u8; 4], id: FourCC| {
                    arcs.iter().rev().find_map(|a| {
                        a.find_tag_by_id(FourCC::new(group), id)
                            .and_then(|e| a.get_tag_data(e).ok())
                    })
                };
                let ids = |v: &[String]| -> anyhow::Result<Vec<FourCC>> {
                    v.iter()
                        .map(|m| {
                            FourCC::try_from(m.as_str())
                                .map_err(|e| anyhow::anyhow!("tag id '{m}': {e:?}"))
                        })
                        .collect()
                };
                let points = point
                    .iter()
                    .map(|p| {
                        let (x, y) = p
                            .split_once(',')
                            .ok_or_else(|| anyhow::anyhow!("point '{p}': want X,Y"))?;
                        Ok((x.trim().parse()?, y.trim().parse()?))
                    })
                    .collect::<anyhow::Result<Vec<(i32, i32)>>>()?;
                movemark_report::movemark_report(
                    &tags,
                    &ids(&inte)?,
                    &ids(&meshes)?,
                    &points,
                    radius,
                    grid,
                )?;
                return Ok(());
            }
            Commands::MediaObjects { archive, mesh_id } => {
                let arc = open_archive(&archive)?;
                let entry = find_tag_any(&arc, "mesh", &mesh_id)
                    .ok_or_else(|| anyhow::anyhow!("No mesh '{}'", mesh_id))?;
                return media_report::media_objects(&arc, entry);
            }
        }
    }

    if args.archive.is_empty() {
        println!("No archive specified. Use --help for usage instructions.");
        return Ok(());
    }

    let archive = open_archive(&args.archive)?;

    if let Some(ref mesh_id) = args.inspect_mesh {
        return inspect_mesh(&archive, mesh_id);
    }
    if let Some(ref stli_id) = args.inspect_strings {
        return inspect_strings(&archive, stli_id);
    }
    if let Some(ref coll_id) = args.inspect_coll {
        return inspect_coll(&archive, coll_id);
    }
    if let Some(ref mons_id) = args.inspect_selection {
        let coll_arc = args.coll_archive.as_deref().map(open_archive).transpose()?;
        return inspect_selection(&args.archive, &archive, mons_id, coll_arc.as_ref());
    }
    if let Some(ref unit_id) = args.inspect_unit {
        let coll_arc = args.coll_archive.as_deref().map(open_archive).transpose()?;
        return inspect_unit(&archive, unit_id, coll_arc.as_ref());
    }
    if let Some(ref id) = args.inspect_proj {
        return inspect_proj(&archive, id);
    }
    if let Some(ref id) = args.inspect_prgr {
        return inspect_prgr(&archive, id);
    }

    let header = archive.header();
    println!("================================================================================");
    println!(" Archive:       {}", args.archive);
    println!(" Name:          {}", header.name_as_str());
    println!(" Description:   {}", header.description_as_str());
    println!(
        " Signature:     {} (0x{:08X})",
        header.signature,
        header.signature.as_u32()
    );
    println!(" Version:       {}", header.version);
    println!(" Archive Size:  {} bytes", header.size);
    println!(" Entry Tags:    {}", header.entry_tag_count);
    println!(" Total Tags:    {}", header.tag_list_count);
    println!("================================================================================");

    if !archive.entry_tags().is_empty() {
        println!();
        println!("Mission Entry Tags ({}):", archive.entry_tags().len());
        println!("  Idx | ID   | Display Name                     | Internal Name");
        println!(
            "  ----+------+----------------------------------+----------------------------------"
        );
        for (i, entry) in archive.entry_tags().iter().enumerate() {
            println!(
                "  {:3} | {:4} | {:32} | {:32}",
                i,
                entry.tag_id.as_str(),
                entry.display_name_as_str(),
                entry.internal_name_as_str(),
            );
        }
        println!();
    }

    if let Some(ref tag_id_str) = args.extract_id {
        let tag_id: FourCC = tag_id_str.as_str().try_into().map_err(|_| {
            anyhow::anyhow!("Tag ID must be exactly 4 characters, got '{}'", tag_id_str)
        })?;

        let entry = archive
            .list_tags()
            .iter()
            .find(|t| {
                t.tag_id == tag_id
                    && args
                        .filter_type
                        .as_deref()
                        .is_none_or(|ty| t.tag_type.to_string().eq_ignore_ascii_case(ty))
            })
            .ok_or_else(|| anyhow::anyhow!("Tag with ID '{}' not found in archive", tag_id_str))?;

        let data = archive.get_tag_data(entry)?;
        println!(
            "Extracted tag '{}' ({}, id: {}): {} bytes",
            entry.name,
            entry.tag_type,
            entry.tag_id,
            data.len()
        );

        if let Some(ref out_path) = args.output {
            fs::write(out_path, &data)?;
            println!("Wrote {} bytes to {}", data.len(), out_path);
        }
        return Ok(());
    }

    if args.info_only {
        return Ok(());
    }

    let mut filtered: Vec<_> = archive.list_tags().iter().collect();

    if let Some(ref type_filter) = args.filter_type {
        filtered.retain(|t| t.tag_type.as_str().eq_ignore_ascii_case(type_filter));
    }

    if let Some(ref name_filter) = args.filter_name {
        let name_lower = name_filter.to_ascii_lowercase();
        filtered.retain(|t| t.name.to_ascii_lowercase().contains(&name_lower));
    }

    let total_matched = filtered.len();
    if let Some(limit) = args.limit {
        filtered.truncate(limit);
    }

    println!();
    println!("Index | Type | ID   | Offset     | Size       | Name");
    println!(
        "------+------+------+------------+------------+------------------------------------------"
    );
    for entry in &filtered {
        println!(
            "{:5} | {:4} | {:4} | {:10} | {:10} | {}",
            entry.index,
            entry.tag_type_str(),
            entry.tag_id_str(),
            entry.offset,
            entry.size,
            entry.name,
        );
    }
    println!(
        "------+------+------+------------+------------+------------------------------------------"
    );
    println!(
        "Showing {} of {} matching tags (archive total: {} tags)",
        filtered.len(),
        total_matched,
        archive.tag_count()
    );

    Ok(())
}

fn open_archive(path_str: &str) -> anyhow::Result<TagArchive> {
    let p = Path::new(path_str);
    if !p.exists() {
        anyhow::bail!("Archive file not found: {}", path_str);
    }
    let vfs = NativeVfs::default_cwd();
    TagArchive::open(&vfs, path_str)
        .map_err(|e| anyhow::anyhow!("Failed to open archive '{}': {}", path_str, e))
}

fn find_tag_any<'a>(
    archive: &'a TagArchive,
    tag_type: &str,
    id_or_name: &str,
) -> Option<&'a TagEntry> {
    let fourcc_type = FourCC::try_from(tag_type).ok();
    let id_trimmed = id_or_name.trim();

    archive
        .list_tags()
        .iter()
        .find(|t| {
            let type_match = fourcc_type.is_none_or(|ft| t.tag_type == ft);
            let tag_id_str = t.tag_id.as_str().trim();
            type_match
                && (tag_id_str.eq_ignore_ascii_case(id_trimmed)
                    || t.name.eq_ignore_ascii_case(id_or_name))
        })
        .or_else(|| {
            archive.list_tags().iter().find(|t| {
                let type_match = fourcc_type.is_none_or(|ft| t.tag_type == ft);
                type_match
                    && t.name
                        .to_ascii_lowercase()
                        .contains(&id_or_name.to_ascii_lowercase())
            })
        })
}

fn inspect_mesh(archive: &TagArchive, mesh_id: &str) -> anyhow::Result<()> {
    let entry = find_tag_any(archive, "mesh", mesh_id)
        .ok_or_else(|| anyhow::anyhow!("Mesh tag '{}' not found in archive", mesh_id))?;

    let bytes = archive.get_tag_data(entry)?;
    let mesh = MeshTag::from_bytes(&bytes)?;

    println!("================================================================================");
    println!(" Mesh Inspector: '{}' (id: {})", entry.name, entry.tag_id);
    println!("================================================================================");
    println!(" Landscape Tag:       {}", mesh.landscape_tag);
    println!(" Media (Water) Tag:    {}", mesh.media_tag);
    println!(
        " Submesh Layout:       {} x {} submeshes",
        mesh.submesh_w, mesh.submesh_h
    );
    if let Some(stli) = mesh.string_list_tag {
        println!(" String List Tag:      {}", stli);
    }
    if let Some(oh) = mesh.overhead_tag {
        println!(" Overhead Map Tag:     {}", oh);
    }
    if let Some(pre) = mesh.pregame_story_tag {
        println!(" Pregame Story Tag:    {}", pre);
    }
    if let Some(post) = mesh.postgame_tag {
        println!(" Postgame Tag:         {}", post);
    }
    if let Some(next) = mesh.next_mesh_id {
        println!(" Next Mesh ID:         {}", next);
    }
    println!(
        " Dimensions (cells):   {} x {}",
        mesh.cell_width, mesh.cell_height
    );
    println!(" Total Cells:          {}", mesh.total_cells);

    let (min_elev, max_elev) = mesh
        .cells
        .iter()
        .fold((i16::MAX, i16::MIN), |(min, max), c| {
            (min.min(c.physical_height), max.max(c.physical_height))
        });
    println!(" Elevation Range:      {} .. {}", min_elev, max_elev);

    let mut terrain = [0usize; 16];
    for c in &mesh.cells {
        let (hi, lo) = c.terrain_types();
        terrain[hi as usize] += 1;
        terrain[lo as usize] += 1;
    }
    let killed = mesh.cells.iter().filter(|c| c.flags & 0x4000 != 0).count();
    let types: Vec<String> = terrain
        .iter()
        .enumerate()
        .filter(|(_, n)| **n > 0)
        .map(|(t, n)| format!("{t}:{n}"))
        .collect();
    println!(" Terrain triangles:    {}", types.join(" "));
    println!(" Killed cells:         {killed}");

    Ok(())
}

fn inspect_strings(archive: &TagArchive, tag_id: &str) -> anyhow::Result<()> {
    let entry = find_tag_any(archive, "stli", tag_id)
        .or_else(|| find_tag_any(archive, "text", tag_id))
        .ok_or_else(|| anyhow::anyhow!("String list / text tag '{}' not found", tag_id))?;

    let bytes = archive.get_tag_data(entry)?;
    let string_list = StringListTag::from_bytes(&bytes)?;

    println!("================================================================================");
    println!(
        " String List Inspector: '{}' ({}, id: {})",
        entry.name, entry.tag_type, entry.tag_id
    );
    println!(" Total Strings: {}", string_list.len());
    println!("================================================================================");
    for (i, s) in string_list.iter().enumerate() {
        println!(" [{:3}] {}", i, s);
    }
    Ok(())
}

fn print_sequences(coll: &CollectionTag) {
    println!(" Sequences:      {}", coll.sequences.len());
    println!(
        " Idx | {:<18} | flags    | views (stored) | frames | ticks | key | loop | bitmaps of frame 0 per view",
        "name"
    );
    for seq in &coll.sequences {
        let views: Vec<String> = (0..seq.num_views as usize)
            .map(|v| {
                seq.bitmap_index(v, 0)
                    .map_or("-".to_string(), |b| b.to_string())
            })
            .collect();
        println!(
            " {:3} | {:<18} | {:08X} | {:5} ({:6}) | {:6} | {:5} | {:3} | {:4} | {}",
            seq.index,
            seq.name,
            seq.flags,
            seq.num_views,
            displayed_views(seq.num_views as usize),
            seq.frames_per_view,
            seq.ticks_per_frame,
            seq.key_frame,
            seq.loop_frame,
            views.join(" ")
        );
    }
}

fn inspect_coll(archive: &TagArchive, coll_id: &str) -> anyhow::Result<()> {
    let entry = find_tag_any(archive, ".256", coll_id)
        .or_else(|| find_tag_any(archive, "coll", coll_id))
        .ok_or_else(|| anyhow::anyhow!("Collection tag '{}' not found", coll_id))?;

    let bytes = archive.get_tag_data(entry)?;
    let coll = CollectionTag::from_bytes(&bytes)?;

    println!("================================================================================");
    println!(
        " Collection Inspector: '{}' (id: {})",
        entry.name, entry.tag_id
    );
    println!(" Bitmap Frames:  {}", coll.bitmap_count);
    println!(" Color Palette:  {} colors", coll.palette.len());
    println!("================================================================================");
    for frame in &coll.frames {
        println!(
            " Frame {:2}: '{:32}' | Size: {}x{} px | Pixels: {} bytes",
            frame.index, frame.name, frame.width, frame.height, frame.pixel_data_size
        );
    }
    print_sequences(&coll);
    Ok(())
}

fn inspect_unit(
    archive: &TagArchive,
    unit_id: &str,
    coll_archive: Option<&TagArchive>,
) -> anyhow::Result<()> {
    let entry = find_tag_any(archive, "unit", unit_id)
        .or_else(|| find_tag_any(archive, "mons", unit_id))
        .ok_or_else(|| anyhow::anyhow!("Unit / monster tag '{}' not found", unit_id))?;

    let unit = if entry.tag_type == "unit" {
        UnitTag::from_archive(archive, entry)?
    } else {
        let bytes = archive.get_tag_data(entry)?;
        UnitTag::from_mons_bytes(&bytes)?
    };

    println!("================================================================================");
    println!(
        " Unit / Monster Inspector: '{}' (id: {})",
        entry.name, entry.tag_id
    );
    println!("================================================================================");
    println!(" Monster Tag ID:       {}", unit.monster_tag_id);
    println!(" Animation Collection: {}", unit.animation_collection);
    println!(" Vitality / Health:    {}", unit.vitality);
    println!(" Movement Speed:       {}", unit.movement_speed);
    println!(" Unit Size:            {}", unit.size);
    if let Some(obj) = unit.object_tag {
        println!(" Object Tag:           {}", obj);
    }
    for (slot, (snd, kind)) in unit.sounds.iter().zip(unit.sound_types).enumerate() {
        if let Some(snd) = snd {
            println!(" Sound Slot {slot}:         {snd} (type {kind})");
        }
    }
    if let Some(proj) = unit.projectile_tag {
        println!(" Projectile Tag:       {}", proj);
    }
    for (i, proj) in unit.attack_projectiles.iter().enumerate() {
        println!(" Attack {i} Projectile:  {}", opt_tag(*proj));
    }
    println!(" Impact Group:         {}", opt_tag(unit.impact_group));
    println!(
        " Standing / Moving:    {} / {}",
        unit.standing_sequence, unit.moving_sequence
    );
    let coll = coll_archive
        .and_then(|a| {
            let e = find_tag_any(a, ".256", unit.animation_collection.as_str())
                .or_else(|| find_tag_any(a, "core", unit.animation_collection.as_str()))?;
            CollectionTag::from_bytes(&a.get_tag_data(e).ok()?).ok()
        })
        .or_else(|| {
            let e = find_tag_any(archive, "core", unit.animation_collection.as_str())?;
            CollectionTag::from_bytes(&archive.get_tag_data(e).ok()?).ok()
        });
    for (slot, &seq) in unit.sequences.iter().enumerate() {
        let name = usize::try_from(seq)
            .ok()
            .and_then(|i| coll.as_ref()?.sequence(i))
            .map_or(String::new(), |s| {
                format!(
                    "'{}' ({} views, {} frames, {} ticks, key {}, loop {})",
                    s.name,
                    s.num_views,
                    s.frames_per_view,
                    s.ticks_per_frame,
                    s.key_frame,
                    s.loop_frame
                )
            });
        println!(" Sequence Slot {slot:2}:     {seq:3} {name}");
    }
    Ok(())
}

fn opt_tag(tag: Option<FourCC>) -> String {
    tag.map_or_else(|| "-".to_string(), |t| t.to_string())
}

fn inspect_proj(archive: &TagArchive, id: &str) -> anyhow::Result<()> {
    let entry = find_tag_any(archive, "proj", id)
        .ok_or_else(|| anyhow::anyhow!("Projectile tag '{}' not found", id))?;
    let p = ProjectileDefinition::from_bytes(entry.tag_id, &archive.get_tag_data(entry)?)?;
    println!("================================================================================");
    println!(
        " Projectile Inspector: '{}' (id: {})",
        entry.name, entry.tag_id
    );
    println!("================================================================================");
    println!(" Flags:                {:#010x}", p.flags);
    println!(" Detonates At Creation: {}", p.detonates_at_creation());
    println!(" Collection:           {}", opt_tag(p.collection));
    println!(" Sequences:            {:?}", p.sequences);
    println!(" Detonation Group:     {}", opt_tag(p.detonation_group));
    println!(" Object:               {}", opt_tag(p.object));
    println!(" Flight Sound:         {}", opt_tag(p.flight_sound));
    println!(" Rebound Sound:        {}", opt_tag(p.rebound_sound));
    println!(" Impact Frequency:     {:#06x}", p.impact_frequency);
    println!(
        " proj +0x14 / +0x5c:   {} / {}",
        opt_tag(p.projectile_14),
        opt_tag(p.projectile_5c)
    );
    println!(
        " prgr +0x60/+0x7c/+0x80: {} / {} / {}",
        opt_tag(p.groups[0]),
        opt_tag(p.groups[1]),
        opt_tag(p.groups[2])
    );
    let d = p.damage;
    let e = p.effective_damage();
    println!(
        " Damage (+0x68):       type {} flags {:#06x} (in memory {:#06x}) amount {} + {} \
         radius {} + {} words {} {}",
        d.damage_type,
        d.flags,
        e.flags,
        d.damage_lower_bound,
        d.damage_delta,
        d.radius_lower_bound,
        d.radius_delta,
        d.word_0c,
        d.word_0e
    );
    Ok(())
}

fn inspect_prgr(archive: &TagArchive, id: &str) -> anyhow::Result<()> {
    let entry = find_tag_any(archive, "prgr", id)
        .ok_or_else(|| anyhow::anyhow!("Projectile group tag '{}' not found", id))?;
    let g = ProjectileGroupDefinition::from_bytes(entry.tag_id, &archive.get_tag_data(entry)?)?;
    println!("================================================================================");
    println!(
        " Projectile Group Inspector: '{}' (id: {})",
        entry.name, entry.tag_id
    );
    println!("================================================================================");
    println!(" Flags:                {:#010x}", g.flags);
    println!(" Mesh Effect:          {}", opt_tag(g.mesh_effect));
    println!(" Launch Sound:         {}", opt_tag(g.sound));
    println!(" Local Group:          {}", opt_tag(g.local_group));
    for (i, part) in g.parts.iter().enumerate() {
        let raw: Vec<String> = part.raw.iter().map(|b| format!("{b:02x}")).collect();
        println!(
            " Part {i:2}:              {} (+0x18 {}) raw {}",
            opt_tag(part.projectile),
            opt_tag(part.projectile_18),
            raw.join("")
        );
    }
    Ok(())
}

fn inspect_selection(
    archive_path: &str,
    archive: &TagArchive,
    mons_id: &str,
    coll_archive: Option<&TagArchive>,
) -> anyhow::Result<()> {
    let entry = find_tag_any(archive, "mons", mons_id)
        .ok_or_else(|| anyhow::anyhow!("Monster tag '{}' not found", mons_id))?;
    let unit = UnitTag::from_mons_bytes(&archive.get_tag_data(entry)?)?;
    println!("================================================================================");
    println!(
        " Selection Inspector: '{}' (mons {}, collection {})",
        entry.name, entry.tag_id, unit.animation_collection
    );
    println!("================================================================================");

    if let Some(obj_id) = unit.object_tag {
        match archive.find_tag_by_id(FourCC::new(*b"obje"), obj_id) {
            Some(obj_entry) => {
                let def = ObjectDefinition::from_bytes(obj_id, &archive.get_tag_data(obj_entry)?)?;
                println!(" Object definition '{}' ({})", obj_entry.name, obj_id);
                println!(
                    "   flags 0x{:04x} (selection frame: {}, vitality bar: {})",
                    def.flags,
                    def.has_selection_frame(),
                    def.has_vitality_bar()
                );
                println!(
                    "   vitality {} (+ random {}), scale 0x{:04x} (+ random {}) = {:.3}",
                    def.vitality,
                    def.vitality_delta,
                    def.scale,
                    def.scale_delta,
                    def.scale as f32 / 256.0
                );
            }
            None => println!(" Object definition {obj_id} not found"),
        }
    }

    let coll = coll_archive.or(Some(archive)).and_then(|a| {
        let e = find_tag_any(a, ".256", unit.animation_collection.as_str())?;
        CollectionTag::from_bytes(&a.get_tag_data(e).ok()?).ok()
    });
    match coll {
        Some(coll) => {
            println!(" Sequence selection rectangles (pixels -> world units, 512 per cell):");
            println!(
                "   Idx | {:<18} | ptw    | half width     | bottom         | top",
                "name"
            );
            let px = |v: i32| v as f32 / 65536.0;
            for seq in &coll.sequences {
                let b = seq.bounds;
                let w = b.world();
                println!(
                    "   {:3} | {:<18} | {:6.3} | {:6.2} -> {:4} | {:6.2} -> {:4} | {:6.2} -> {:4}",
                    seq.index,
                    seq.name,
                    px(b.pixels_to_world),
                    px(b.half_width),
                    w.half_width,
                    px(b.bottom),
                    w.bottom,
                    px(b.top),
                    w.top
                );
            }
        }
        None => println!(" Collection {} not found", unit.animation_collection),
    }

    let coll_type = FourCC::new(*b".256");
    let decode = |arc: &TagArchive| -> Option<InterfaceColorTable> {
        let e = arc.find_tag_by_id(coll_type, STANDARD_INTERFACE_COLLECTION)?;
        let coll = CollectionTag::from_bytes(&arc.get_tag_data(e).ok()?).ok()?;
        InterfaceColorTable::from_collection(&coll)
    };
    let colors = decode(archive).or_else(|| {
        let dir = Path::new(archive_path).parent()?;
        ["international small install", "international large install"]
            .iter()
            .map(|n| dir.join(n))
            .filter(|p| p.exists())
            .filter_map(|p| open_archive(&p.to_string_lossy()).ok())
            .find_map(|a| decode(&a))
    });
    match colors {
        Some(t) => {
            let hex = |i: usize| {
                let c = t.color(i);
                format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
            };
            println!(" Interface colors ('stin' bitmap #0, get_interface_color):");
            for (i, what) in [
                (COLOR_SELECTION_OWNED, "selection frame, owned"),
                (COLOR_SELECTION_ENEMY, "selection frame, enemy"),
                (COLOR_SELECTION_ALLY, "selection frame, ally"),
                (COLOR_SELECTION_OBJECT, "selection frame, object"),
            ] {
                println!("   [{i:2}] {} {what}", hex(i));
            }
            println!("   vitality step | fill (0x0c+step) | background (0x22+step)");
            for step in 0..=VITALITY_COLOR_STEPS {
                println!(
                    "   {step:13} | {:16} | {}",
                    hex(COLOR_VITALITY_FILL_BASE + step),
                    hex(COLOR_VITALITY_BACKGROUND_BASE + step)
                );
            }
        }
        None => println!(" Interface collection 'stin' not found"),
    }
    Ok(())
}

fn export_obj(
    archive: &TagArchive,
    mesh_id: &str,
    output: &str,
    height_scale: f32,
) -> anyhow::Result<()> {
    let entry = find_tag_any(archive, "mesh", mesh_id)
        .ok_or_else(|| anyhow::anyhow!("Mesh tag '{}' not found in archive", mesh_id))?;
    let data = archive.get_tag_data(entry)?;
    let mesh = MeshTag::from_bytes(&data)?;
    let out_path = std::path::Path::new(output);
    mesh.export_obj(out_path, height_scale)?;
    println!(
        "Successfully exported mesh '{}' ({}x{} cells, {} vertices) to '{}'",
        entry.name,
        mesh.cell_width,
        mesh.cell_height,
        (mesh.cell_width as usize + 1) * (mesh.cell_height as usize + 1),
        output
    );
    Ok(())
}
