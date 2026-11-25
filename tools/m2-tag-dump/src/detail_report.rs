use mandrake_common::FourCC;
use mandrake_tag::{DetailMap, DetailTexture, MeshTag, TagArchive};

pub fn run(mesh: &str, data: &[u8], dmap_arc: &TagArchive) -> anyhow::Result<()> {
    let mesh_tag = MeshTag::from_bytes(data)?;
    let coll = mesh_tag.landscape_tag;
    println!(
        "mesh '{mesh}': landscape collection '{coll}', {} x {} sub-meshes = {} x {} cells",
        mesh_tag.submesh_w, mesh_tag.submesh_h, mesh_tag.cell_width, mesh_tag.cell_height
    );
    let Some(dentry) = dmap_arc.find_tag_by_id(FourCC::new(*b"dmap"), coll) else {
        println!("no dmap '{coll}' in this archive");
        return Ok(());
    };
    let map = DetailMap::from_bytes(&dmap_arc.get_tag_data(dentry)?)?;
    let fits = map.matches_mesh(mesh_tag.submesh_w, mesh_tag.submesh_h);
    println!(
        "dmap '{coll}' ({}): {} x {} cells, {}",
        dentry.name,
        map.width,
        map.height,
        if fits {
            "matches the mesh"
        } else {
            "does not match the mesh: the engine drops it"
        }
    );
    let forced = map.triangle_entries.iter().filter(|&&b| b != 0).count();
    println!(
        "triangles with a forced entry (second stream): {forced} of {}",
        map.triangle_entries.len()
    );
    println!("entry  dtex    repeat  colour   cells  triangles  texture              name");
    for (i, e) in map.entries.iter().enumerate() {
        let cells = map
            .cell_entries
            .iter()
            .filter(|&&c| c as usize == i)
            .count();
        let tris = map
            .triangle_entries
            .iter()
            .filter(|&&c| c as usize == i)
            .count();
        let Some(t) = e.texture else {
            if cells > 0 || (tris > 0 && i != 0) {
                println!("{i:5}  (none)  used by {cells} cells, {tris} triangles");
            }
            continue;
        };
        let tex = match dmap_arc.find_tag_by_id(FourCC::new(*b"dtex"), t) {
            None => "missing".to_string(),
            Some(te) => {
                let img = DetailTexture::from_bytes(&dmap_arc.get_tag_data(te)?)?.decode()?;
                format!("{} x {} primary {}", img.width, img.height, img.primary)
            }
        };
        let [r, g, b] = e.color;
        println!(
            "{i:5}  {:<6}  {:6}  {r:02x}{g:02x}{b:02x}   {cells:5}  {tris:9}  {tex:<19}  {}",
            format!("'{t}'"),
            e.repeat,
            e.name
        );
    }
    Ok(())
}
