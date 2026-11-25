use mandrake_view::{camera, landscape, media, model_surfaces};

pub mod billboard;
pub mod detail;
pub mod frame_canvas;
pub mod interface_frame;
pub mod interface_present;
pub mod interface_scale;
pub mod mesh;
#[cfg(test)]
mod mesh_pick_tests;
pub mod model;
pub mod overlay;
pub mod pipeline;
pub mod reflection_edge;
pub mod rife46;
pub mod screen_fade;
#[cfg(test)]
#[path = "../../mandrake-common/tests/support/require_data.rs"]
pub(crate) mod test_data;
pub mod uniforms;
pub mod vertex;
pub mod water;

pub use billboard::{
    BillboardRenderer, TextureAtlas, create_atlas_from_collection, create_atlas_from_images,
    create_atlas_from_images_with_offsets, create_texture_from_rgba, frame_to_rgba_image,
    grow_atlas_from_images_with_offsets,
};
pub use frame_canvas::{CANVAS_FORMAT, FrameCanvas};
pub use interface_frame::{DrawInterface, InterfaceFrame};
pub use interface_present::{InterfacePresenter, MenuCanvasGpu};
pub use interface_scale::{InterfaceScaler, ScrollDraw, set_lanczos_sharpen};
pub use mesh::{MediaVertex, OccluderRule, TerrainMesh};
pub use model::{
    ModelMesh, ModelRenderer, ModelSubMesh, ModelSubmeshes, ModelUniforms, ReflectionBatch,
    ReflectionRun, TranslucentBatch, TranslucentTriangle, TrianglePassState,
    sort_translucent_triangles, sort_translucent_triangles_on_screen,
};
pub use overlay::{OverlayDraw, OverlayImage, OverlayLayer, OverlayRenderer};
pub use pipeline::{LandscapeTexture, RenderPipelineConfig, TerrainRenderer};
pub use reflection_edge::ReflectionEdge;
pub use screen_fade::ScreenFade;
pub use uniforms::{MAX_LIGHTS, PointLight, SceneUniforms, TextureSampling};
pub use vertex::{OccluderVertex, TerrainVertex};

pub fn scene_constants(format: wgpu::TextureFormat) -> &'static [(&'static str, f64)] {
    if format.is_srgb() {
        &[]
    } else {
        &[("SCENE_ENCODE_SRGB", 1.0)]
    }
}

pub fn is_ten_bit(format: wgpu::TextureFormat) -> bool {
    matches!(format, wgpu::TextureFormat::Rgb10a2Unorm | CANVAS_FORMAT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_view::camera::Camera;

    #[test]
    fn test_terrain_wgsl_syntax_validation() {
        let terrain_src = TerrainRenderer::TERRAIN_SHADER_SRC;
        let module = naga::front::wgsl::parse_str(terrain_src);
        assert!(
            module.is_ok(),
            "Terrain WGSL parse failed: {:?}",
            module.err()
        );
    }

    #[test]
    fn test_occluder_wgsl_syntax_validation() {
        let module = naga::front::wgsl::parse_str(TerrainRenderer::OCCLUDER_SHADER_SRC);
        assert!(
            module.is_ok(),
            "Occluder WGSL parse failed: {:?}",
            module.err()
        );
    }

    #[test]
    fn test_billboard_wgsl_syntax_validation() {
        let billboard_src = BillboardRenderer::BILLBOARD_SHADER_SRC;
        let module = naga::front::wgsl::parse_str(billboard_src);
        assert!(
            module.is_ok(),
            "Billboard WGSL parse failed: {:?}",
            module.err()
        );
    }

    #[test]
    fn test_model_wgsl_syntax_validation() {
        let model_src = ModelRenderer::MODEL_SHADER_SRC;
        let module = naga::front::wgsl::parse_str(model_src);
        assert!(
            module.is_ok(),
            "Model WGSL parse failed: {:?}",
            module.err()
        );
    }

    #[test]
    fn test_pipeline_config_default() {
        let config = RenderPipelineConfig::default();
        assert_eq!(config.width, 1280);
        assert_eq!(config.height, 720);
        assert_eq!(config.target_format, wgpu::TextureFormat::Rgba8UnormSrgb);
        assert_eq!(config.depth_format, Some(wgpu::TextureFormat::Depth32Float));
    }

    #[test]
    fn test_camera_and_scene_uniforms_integration() {
        let camera = Camera::default();
        let uniforms = SceneUniforms::from_camera(&camera, 16.0 / 9.0);

        let eye = camera.eye_position();
        assert_eq!(uniforms.camera_pos[0], eye[0]);
        assert_eq!(uniforms.camera_pos[1], eye[1]);
        assert_eq!(uniforms.camera_pos[2], eye[2]);
        assert_eq!(uniforms.camera_pos[3], 1.0);

        assert_ne!(uniforms.view_proj[0][0], 0.0);
        assert_ne!(uniforms.view_proj[1][1], 0.0);
        assert_ne!(uniforms.view_proj[2][2], 0.0);
    }
}
