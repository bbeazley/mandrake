#![allow(dead_code)]

#[path = "../../../mandrake-common/tests/support/require_data.rs"]
pub mod test_data;

use std::path::Path;

use mandrake_common::{FourCC, NativeVfs};
use mandrake_render::detail::LandscapeDetail;
use mandrake_tag::mesh::MeshLod;
use mandrake_tag::{DetailMap, DetailTexture, MeshTag, TagArchive};

pub const DETAIL_MEGAPACK: &str = "Magma - Detail Texture Megapack";

pub struct Game {
    pub archives: Vec<TagArchive>,
    pub detail: TagArchive,
}

impl Game {
    pub fn open() -> Option<Self> {
        let pack = format!(
            "{}/plugins/{DETAIL_MEGAPACK}",
            test_data::game_dir().display()
        );
        if !test_data::require_data(Path::new(&pack)) {
            return None;
        }
        let files = mandrake_tag::engine_tag_files(
            &test_data::game_dir(),
            &["Patch 1.8.5 Maps", "Patch 1.8.5 Unity"],
        );
        if !test_data::require(files.complete(), "the engine's tag set") {
            return None;
        }
        let vfs = NativeVfs::default_cwd();
        let archives = files
            .paths
            .iter()
            .filter_map(|p| TagArchive::open(&vfs, &p.to_string_lossy()).ok())
            .collect();
        let detail = TagArchive::open(&vfs, &pack).ok()?;
        Some(Self { archives, detail })
    }

    pub fn tag(&self, group: &[u8; 4], id: FourCC) -> Option<Vec<u8>> {
        self.archives.iter().rev().find_map(|a| {
            a.find_tag_by_id(FourCC::new(*group), id)
                .and_then(|e| a.get_tag_data(e).ok())
        })
    }

    pub fn mesh(&self, id: &[u8; 4]) -> (MeshTag, MeshLod, Vec<u8>) {
        let data = self.tag(b"mesh", FourCC::new(*id)).expect("mesh");
        let mesh = MeshTag::from_bytes(&data).unwrap();
        let lod = MeshLod::from_mesh_bytes(&data, mesh.cell_width, mesh.cell_height).unwrap();
        (mesh, lod, data)
    }

    pub fn dmap(&self, mesh: &MeshTag) -> DetailMap {
        let e = self
            .detail
            .find_tag_by_id(FourCC::new(*b"dmap"), mesh.landscape_tag)
            .expect("dmap");
        let map = DetailMap::from_bytes(&self.detail.get_tag_data(e).unwrap()).unwrap();
        assert!(map.matches_mesh(mesh.submesh_w, mesh.submesh_h));
        map
    }

    pub fn landscape_detail(&self, mesh: &MeshTag) -> LandscapeDetail {
        LandscapeDetail::new(&self.dmap(mesh), |id| {
            let e = self.detail.find_tag_by_id(FourCC::new(*b"dtex"), id)?;
            DetailTexture::from_bytes(&self.detail.get_tag_data(e).ok()?)
                .ok()?
                .decode()
                .ok()
        })
        .expect("every used dtex")
    }
}

pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

pub fn gpu(label: &str, features: wgpu::Features) -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .ok()?;
    eprintln!("adapter: {:?}", adapter.get_info().name);
    block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some(label),
        required_features: adapter.features() & features,
        required_limits: adapter.limits(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
    }))
    .ok()
}
