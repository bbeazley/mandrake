use super::*;

/// What every water draw binds.
pub(super) struct Draw<'a> {
    pub(super) terrain: &'a TerrainRenderer,
    pub(super) mesh: &'a TerrainMesh,
    pub(super) texture_bind_group: Option<&'a wgpu::BindGroup>,
}

impl WaterShading {
    /// `physical`: draw the category-4 billboards `instances` of `instance_buffer` as
    /// [`crate::billboard::BillboardRenderer::render_reflections`] does, but split: with
    /// `submerged` false the reflections only (into the reflection layer; `soft_edge` as there),
    /// with it true the underwater copies and media effects only (into the submerged layer).
    /// Returns false without the split (the caller draws them as before).
    #[allow(clippy::too_many_arguments)]
    pub fn render_category4_billboards(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        billboards: &crate::billboard::BillboardRenderer,
        uniform_bind_group: &wgpu::BindGroup,
        texture_bind_group: Option<&wgpu::BindGroup>,
        instance_buffer: &wgpu::Buffer,
        instances: std::ops::Range<u32>,
        soft_edge: Option<&wgpu::BindGroup>,
        submerged: bool,
    ) -> bool {
        let Some(c4) = &self.category4 else {
            return false;
        };
        if instances.is_empty() {
            return true;
        }
        match (submerged, soft_edge) {
            (true, _) => pass.set_pipeline(&c4.submerged),
            (false, Some(edge)) => {
                pass.set_pipeline(&c4.soft_reflection);
                pass.set_bind_group(2, edge, &[]);
            }
            (false, None) => pass.set_pipeline(&c4.reflection),
        }
        pass.set_bind_group(0, uniform_bind_group, &[]);
        pass.set_bind_group(
            1,
            texture_bind_group.unwrap_or(&billboards.default_texture_bind_group),
            &[],
        );
        pass.set_vertex_buffer(0, instance_buffer.slice(..));
        pass.draw(0..6, instances);
        true
    }

    /// The scene pass, in place of [`TerrainRenderer::render_terrain_media`]: the lowered
    /// terrain's depth and an empty layer under the water, a media triangle's terrain above it.
    pub fn render_bed<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        terrain: &'a TerrainRenderer,
        mesh: &'a TerrainMesh,
        texture_bind_group: Option<&'a wgpu::BindGroup>,
    ) {
        let Some(lowered) = &self.lowered else {
            return;
        };
        let draw = Draw {
            terrain,
            mesh,
            texture_bind_group,
        };
        let (media_buf, media_range) = self.media_draw(mesh);
        self.draw(
            pass,
            &self.pipelines.bed_media,
            &draw,
            lowered,
            media_buf,
            media_range,
        );
        let (dry_buf, dry_range) = self.near_dry_draw(mesh);
        self.draw(
            pass,
            &self.pipelines.bed_dry,
            &draw,
            lowered,
            dry_buf,
            dry_range,
        );
    }

    /// After the reflections (category 4), in a pass on the display-value view of the target that
    /// the reflections were drawn in: the reflections' Fresnel share, then the surface and the
    /// fogged terrain under them.
    pub fn render_under<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        terrain: &'a TerrainRenderer,
        mesh: &'a TerrainMesh,
        texture_bind_group: Option<&'a wgpu::BindGroup>,
    ) {
        let Some(lowered) = &self.lowered else {
            return;
        };
        let draw = Draw {
            terrain,
            mesh,
            texture_bind_group,
        };
        let (near_buf, near) = self.near_all_draw(mesh);
        if self.physical {
            self.draw(
                pass,
                &self.pipelines.surface,
                &draw,
                &mesh.vertex_buffer,
                near_buf,
                near.clone(),
            );
        } else {
            self.draw(
                pass,
                &self.pipelines.keep,
                &draw,
                &mesh.vertex_buffer,
                near_buf,
                near.clone(),
            );
            self.draw(
                pass,
                &self.pipelines.surface_under,
                &draw,
                &mesh.vertex_buffer,
                near_buf,
                near.clone(),
            );
        }
        self.draw(
            pass,
            &self.pipelines.bed_under,
            &draw,
            lowered,
            near_buf,
            near,
        );
    }

    /// Wet sand, in the opaque pass right after the lowered dry triangles: the terrain just above
    /// the water level darkened, with a little sheen. Nothing with `wet_sand` off.
    pub fn render_wet<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        terrain: &'a TerrainRenderer,
        mesh: &'a TerrainMesh,
        texture_bind_group: Option<&'a wgpu::BindGroup>,
    ) {
        let (Some(pipeline), Some(lowered)) = (&self.pipelines.wet, &self.lowered) else {
            return;
        };
        let draw = Draw {
            terrain,
            mesh,
            texture_bind_group,
        };
        let (near_buf, near) = self.near_all_draw(mesh);
        self.draw(pass, pipeline, &draw, lowered, near_buf, near);
    }

    /// First in the opaque pass: the surface's depth, which hides the lowered dry triangles under
    /// it and cuts the models as the media triangles did.
    pub fn render_surface_depth<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        terrain: &'a TerrainRenderer,
        mesh: &'a TerrainMesh,
        texture_bind_group: Option<&'a wgpu::BindGroup>,
    ) {
        let draw = Draw {
            terrain,
            mesh,
            texture_bind_group,
        };
        let (near_buf, near) = self.near_all_draw(mesh);
        self.draw(
            pass,
            &self.pipelines.surface_depth,
            &draw,
            &mesh.vertex_buffer,
            near_buf,
            near,
        );
    }

    pub(super) fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        pipeline: &'a wgpu::RenderPipeline,
        draw: &Draw<'a>,
        vertices: &'a wgpu::Buffer,
        index_buffer: &'a wgpu::Buffer,
        indices: std::ops::Range<u32>,
    ) {
        if draw.mesh.num_media_indices == 0 || indices.is_empty() {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &draw.terrain.uniform_bind_group, &[]);
        pass.set_bind_group(
            1,
            draw.texture_bind_group
                .unwrap_or(&draw.terrain.default_texture_bind_group),
            &[],
        );
        pass.set_bind_group(2, &self.pipelines.empty_group, &[]);
        let state = self.ripples.as_ref().map_or(0, |r| r.current());
        pass.set_bind_group(3, &self.bind_groups[state], &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(indices, 0, 0..1);
    }

    fn media_draw<'a>(&'a self, mesh: &'a TerrainMesh) -> (&'a wgpu::Buffer, std::ops::Range<u32>) {
        if let Some(full) = &self.full {
            (&full.indices, 0..full.media)
        } else {
            (&mesh.index_buffer, 0..mesh.num_media_indices)
        }
    }

    fn near_dry_draw<'a>(
        &'a self,
        mesh: &'a TerrainMesh,
    ) -> (&'a wgpu::Buffer, std::ops::Range<u32>) {
        if let Some(full) = &self.full {
            (&full.indices, full.media..full.near)
        } else if let Some(water) = &self.water_indices {
            (&water.indices, water.media..water.near)
        } else {
            (&mesh.index_buffer, mesh.num_media_indices..mesh.num_indices)
        }
    }

    fn near_all_draw<'a>(
        &'a self,
        mesh: &'a TerrainMesh,
    ) -> (&'a wgpu::Buffer, std::ops::Range<u32>) {
        if let Some(full) = &self.full {
            (&full.indices, 0..full.near)
        } else if let Some(water) = &self.water_indices {
            (&water.indices, 0..water.near)
        } else {
            (&mesh.index_buffer, 0..mesh.num_indices)
        }
    }

    /// The dry pass's lowered triangles (in place of
    /// [`TerrainRenderer::render_terrain_dry_with`] on the lowered terrain): at full resolution
    /// on the plain path, else the engine's.
    pub fn render_dry<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        terrain: &'a TerrainRenderer,
        mesh: &'a TerrainMesh,
        texture_bind_group: Option<&'a wgpu::BindGroup>,
    ) {
        let Some(lowered) = &self.lowered else {
            return;
        };
        match &self.full {
            Some(full) => terrain.render_terrain_indexed(
                pass,
                texture_bind_group,
                lowered,
                &full.indices,
                full.media..full.total,
            ),
            None => terrain.render_terrain_dry_with(pass, mesh, texture_bind_group, lowered),
        }
    }
}
