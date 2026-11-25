use crate::camera::Camera;
use crate::interface_scale::{FilterModes, InterfaceFilter};
use bytemuck::{Pod, Zeroable};
use mandrake_tag::MeshFog;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextureSampling {
    Software,
    #[default]
    Linear,
    Bicubic,
    Sharp,
}

const SPRITE_MODE_SOFTWARE: u32 = 4;

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub struct PointLight {
    pub pos: [f32; 4],
    pub color: [f32; 4],
}

impl Default for PointLight {
    fn default() -> Self {
        Self {
            pos: [0.0; 4],
            color: [0.0; 4],
        }
    }
}

pub const MAX_LIGHTS: usize = 16;

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub struct SceneUniforms {
    pub view_proj: [[f32; 4]; 4],
    pub camera_pos: [f32; 4],
    pub render_params: [f32; 4],
    pub fog: [f32; 4],
    pub fog_height: [f32; 4],
    pub fog_motion: [f32; 4],
    pub fog_shape: [f32; 4],
    pub ambient_tint: [f32; 4],
    pub light_params: [u32; 4],
    pub lights: [PointLight; MAX_LIGHTS],
}

impl Default for SceneUniforms {
    fn default() -> Self {
        Self {
            view_proj: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            camera_pos: [0.0, 30.0, 30.0, 1.0],
            render_params: [0.0; 4],
            fog: [0.0; 4],
            fog_height: [0.0; 4],
            fog_motion: [0.0; 4],
            fog_shape: [0.0; 4],
            ambient_tint: [1.0, 1.0, 1.0, 0.0],
            light_params: [0; 4],
            lights: [PointLight::default(); MAX_LIGHTS],
        }
    }
}

impl SceneUniforms {
    pub fn from_camera(camera: &Camera, aspect_ratio: f32) -> Self {
        let vp = camera.view_proj_matrix(aspect_ratio);
        let eye = camera.eye_position();

        Self {
            view_proj: vp.to_cols_array_2d(),
            camera_pos: [eye[0], eye[1], eye[2], 1.0],
            ..Default::default()
        }
    }

    pub fn with_viewport(mut self, width: u32, height: u32) -> Self {
        self.render_params[2] = width.max(1) as f32;
        self.render_params[3] = height.max(1) as f32;
        self
    }

    pub fn with_sampling(mut self, sampling: TextureSampling) -> Self {
        let (scene, sprites) = match sampling {
            TextureSampling::Linear => (0.0, 0.0),
            TextureSampling::Software => (1.0, SPRITE_MODE_SOFTWARE as f32),
            TextureSampling::Bicubic => (2.0, InterfaceFilter::Lanczos2.sprite_mode() as f32),
            TextureSampling::Sharp => (3.0, InterfaceFilter::Sharp.sprite_mode() as f32),
        };
        self.render_params[0] = scene;
        self.render_params[1] = sprites;
        self
    }

    pub fn with_sprite_filter(mut self, filter: InterfaceFilter) -> Self {
        self.render_params[1] = filter.sprite_mode() as f32;
        self
    }

    pub fn with_fog(mut self, fog: &MeshFog) -> Self {
        let c = |v: u16| f32::from(v) / 65535.0;
        self.fog = [
            c(fog.color[0]),
            c(fog.color[1]),
            c(fog.color[2]),
            fog.density() * 512.0,
        ];
        self
    }

    pub fn with_height_fog(
        mut self,
        color: [f32; 3],
        fog_height: [f32; 4],
        fog_motion: [f32; 4],
        fog_shape: [f32; 4],
    ) -> Self {
        self.fog[..3].copy_from_slice(&color);
        self.fog_height = fog_height;
        self.fog_motion = fog_motion;
        self.fog_shape = fog_shape;
        self
    }

    pub fn with_lighting(
        mut self,
        ambient_tint: [f32; 3],
        enabled: bool,
        lights: &[PointLight],
    ) -> Self {
        self.ambient_tint = [
            ambient_tint[0],
            ambient_tint[1],
            ambient_tint[2],
            if enabled { 1.0 } else { 0.0 },
        ];
        let count = lights.len().min(MAX_LIGHTS);
        self.light_params = [count as u32, 0, 0, 0];
        for (dst, src) in self.lights[..count].iter_mut().zip(lights) {
            *dst = *src;
        }
        for dst in &mut self.lights[count..] {
            *dst = PointLight::default();
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uniform_alignment_and_size() {
        assert_eq!(core::mem::size_of::<SceneUniforms>(), 704);
        assert_eq!(core::mem::align_of::<SceneUniforms>(), 4);
        assert_eq!(core::mem::size_of::<SceneUniforms>() % 16, 0);
        let def = SceneUniforms::default();
        let bytes = bytemuck::bytes_of(&def);
        assert_eq!(bytes.len(), 704);
    }

    #[test]
    fn shaders_declare_the_same_struct() {
        for src in [
            crate::pipeline::TerrainRenderer::TERRAIN_SHADER_SRC,
            crate::pipeline::TerrainRenderer::OCCLUDER_SHADER_SRC,
            crate::reflection_edge::ReflectionEdge::SHADER_SRC,
            crate::model::ModelRenderer::MODEL_SHADER_SRC,
            crate::BillboardRenderer::BILLBOARD_SHADER_SRC,
        ] {
            let module = naga::front::wgsl::parse_str(src).unwrap();
            let span = module
                .types
                .iter()
                .find(|(_, t)| t.name.as_deref() == Some("SceneUniforms"))
                .map(|(_, t)| t.inner.size(module.to_ctx()))
                .unwrap();
            assert_eq!(span as usize, core::mem::size_of::<SceneUniforms>());
        }
    }

    #[test]
    fn world_shaders_use_all_max_lights() {
        let expected = format!("min(scene.light_params.x, {MAX_LIGHTS}u)");
        for (name, src) in [
            (
                "terrain",
                crate::pipeline::TerrainRenderer::TERRAIN_SHADER_SRC,
            ),
            ("model", crate::model::ModelRenderer::MODEL_SHADER_SRC),
            ("billboard", crate::BillboardRenderer::BILLBOARD_SHADER_SRC),
        ] {
            assert!(src.contains(&expected), "{name} missing {expected}");
            assert!(
                !src.contains("min(scene.light_params.x, 8u)"),
                "{name} still caps lights at 8"
            );
        }
    }

    #[test]
    fn lighting_uniforms() {
        let def = SceneUniforms::default();
        assert_eq!(def.ambient_tint, [1.0, 1.0, 1.0, 0.0]);
        assert_eq!(def.light_params[0], 0);

        let lights = [PointLight {
            pos: [1.0, 2.0, 3.0, 8.0],
            color: [1.0, 0.5, 0.2, 1.5],
        }];
        let lit = def.with_lighting([0.1, 0.15, 0.2], true, &lights);
        assert_eq!(lit.ambient_tint, [0.1, 0.15, 0.2, 1.0]);
        assert_eq!(lit.light_params[0], 1);
        assert_eq!(lit.lights[0], lights[0]);
    }

    #[test]
    fn fog_from_the_mesh_header() {
        assert_eq!(SceneUniforms::default().fog, [0.0; 4]);
        let cr01 = MeshFog {
            color: [52428; 3],
            density_bits: 3e-5f32.to_bits(),
        };
        let u = SceneUniforms::default().with_fog(&cr01);
        assert_eq!(u.fog[..3], [52428.0 / 65535.0; 3]);
        assert_eq!(u.fog[3], 3e-5f32 * 512.0);
        let none = SceneUniforms::default().with_fog(&MeshFog::default());
        assert_eq!(none.fog, [0.0; 4]);
    }

    #[test]
    fn height_fog_keeps_the_density() {
        let cr01 = MeshFog {
            color: [52428; 3],
            density_bits: 3e-5f32.to_bits(),
        };
        let u = SceneUniforms::default().with_fog(&cr01).with_height_fog(
            [0.25, 0.5, 0.75],
            [1.0, 2.0, 3.0, 4.0],
            [6.0, 0.7, 0.1, 0.2],
            [0.25, 2.0, 0.0, 0.0],
        );
        assert_eq!(u.fog, [0.25, 0.5, 0.75, 3e-5f32 * 512.0]);
        assert_eq!(u.fog_height, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(u.fog_motion, [6.0, 0.7, 0.1, 0.2]);
        assert_eq!(u.fog_shape, [0.25, 2.0, 0.0, 0.0]);
    }

    #[test]
    fn sampling_slots() {
        let d = SceneUniforms::default();
        assert_eq!(d.render_params[0], 0.0);
        let l = d.with_sampling(TextureSampling::Linear);
        assert_eq!(l.render_params[0], 0.0);
        assert_eq!(l.render_params[1], 0.0);
        let s = SceneUniforms::default().with_sampling(TextureSampling::Software);
        assert_eq!(s.render_params[0], 1.0);
        let src = crate::BillboardRenderer::BILLBOARD_SHADER_SRC;
        let line = format!(
            "const SPRITE_FILTER_SOFTWARE: u32 = {}u;",
            s.render_params[1] as u32
        );
        assert!(src.contains(&line), "{line}");
        let e = s.with_sprite_filter(InterfaceFilter::Linear);
        assert_eq!(e.render_params[0], 1.0);
        assert_eq!(e.render_params[1], 0.0);
    }

    #[test]
    fn bicubic_and_sharp_slots() {
        let slots = |s| {
            let p = SceneUniforms::default()
                .with_viewport(640, 480)
                .with_sampling(s)
                .render_params;
            assert_eq!(
                p[2..],
                [640.0, 480.0],
                "the viewport stays in its own slots"
            );
            [p[0], p[1]]
        };
        let bicubic = slots(TextureSampling::Bicubic);
        let sharp = slots(TextureSampling::Sharp);
        assert_eq!(bicubic[1], InterfaceFilter::Lanczos2.sprite_mode() as f32);
        assert_eq!(sharp[1], InterfaceFilter::Sharp.sprite_mode() as f32);
        for src in [
            include_str!("../shaders/terrain.wgsl"),
            include_str!("../shaders/model.wgsl"),
        ] {
            for (name, value) in [("BICUBIC", bicubic[0]), ("SHARP", sharp[0])] {
                let line = format!("const SAMPLING_{name}: u32 = {}u;", value as u32);
                assert!(src.contains(&line), "{line}");
            }
            let line = format!(
                "const SAMPLING_SOFTWARE: u32 = {}u;",
                slots(TextureSampling::Software)[0] as u32
            );
            assert!(src.contains(&line), "{line}");
        }
        let e = SceneUniforms::default()
            .with_sampling(TextureSampling::Bicubic)
            .with_sprite_filter(InterfaceFilter::Nearest);
        assert_eq!(e.render_params[..2], [2.0, 2.0]);
        assert_eq!(slots(TextureSampling::Linear), [0.0, 0.0]);
    }

    #[test]
    fn sprite_filter_slot() {
        let mode = |f| SceneUniforms::default().with_sprite_filter(f).render_params[1];
        assert_eq!(SceneUniforms::default().render_params[1], 0.0);
        assert_eq!(mode(InterfaceFilter::Linear), 0.0);
        let src = crate::BillboardRenderer::BILLBOARD_SHADER_SRC;
        for (filter, name) in [
            (InterfaceFilter::Sharp, "SHARP"),
            (InterfaceFilter::Nearest, "NEAREST"),
            (InterfaceFilter::Lanczos2, "LANCZOS2"),
        ] {
            let line = format!(
                "const SPRITE_FILTER_{name}: u32 = {}u;",
                mode(filter) as u32
            );
            assert!(src.contains(&line), "{line}");
        }
        let u = SceneUniforms::default()
            .with_viewport(640, 480)
            .with_sprite_filter(InterfaceFilter::Lanczos2);
        assert_eq!(u.render_params, [0.0, 3.0, 640.0, 480.0]);
    }
}
