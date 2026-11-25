use crate::app::frame_budget::{GpuTimer, Mark};
use crate::fog::MapFog;
use crate::sprite_manager::TEAM_CACHE_FLAG;
use crate::ui_scale::{Sampling, UiFilter};
use mandrake_render::SceneUniforms;
use mandrake_render::{
    BillboardRenderer, ModelMesh, ModelRenderer, ReflectionBatch, ReflectionEdge, TerrainMesh,
    TerrainRenderer, TranslucentBatch, TrianglePassState,
};
use mandrake_view::billboard::{
    BillboardInstance, Category4Step, SpritePassOrder, SpriteStep, category4_steps,
    merge_sprite_steps, order_for_sprite_pass, place_sprites_in_engine_view, sprite_category_steps,
};
use std::ops::Range;

#[derive(Clone, Copy)]
pub(crate) struct SpriteAtlases<'a> {
    pub(crate) atlas: &'a wgpu::BindGroup,
    pub(crate) team: Option<&'a wgpu::BindGroup>,
}

impl<'a> SpriteAtlases<'a> {
    pub(crate) fn for_each_run(
        self,
        instances: &[BillboardInstance],
        range: Range<u32>,
        mut f: impl FnMut(&'a wgpu::BindGroup, Range<u32>),
    ) {
        if range.is_empty() {
            return;
        }
        let Some(team_bg) = self.team else {
            f(self.atlas, range);
            return;
        };
        let mut run_start = range.start;
        let mut run_team = instances
            .get(run_start as usize)
            .is_some_and(|b| b.flags & TEAM_CACHE_FLAG != 0);
        for i in (range.start + 1)..range.end {
            let team = instances
                .get(i as usize)
                .is_some_and(|b| b.flags & TEAM_CACHE_FLAG != 0);
            if team != run_team {
                f(if run_team { team_bg } else { self.atlas }, run_start..i);
                run_start = i;
                run_team = team;
            }
        }
        f(
            if run_team { team_bg } else { self.atlas },
            run_start..range.end,
        );
    }
}

#[cfg(test)]
fn team_cache_runs(instances: &[BillboardInstance], range: Range<u32>) -> Vec<(bool, Range<u32>)> {
    let mut out: Vec<(bool, Range<u32>)> = Vec::new();
    for i in range {
        let team = instances
            .get(i as usize)
            .is_some_and(|b| b.flags & TEAM_CACHE_FLAG != 0);
        match out.last_mut() {
            Some((t, r)) if *t == team && r.end == i => r.end = i + 1,
            _ => out.push((team, i..i + 1)),
        }
    }
    out
}

pub(crate) struct WorldTargets<'v> {
    pub(crate) color: &'v wgpu::TextureView,
    pub(crate) interface: &'v wgpu::TextureView,
    pub(crate) depth: &'v wgpu::TextureView,
}

pub(crate) type ModelDraw<'r> = (&'r wgpu::BindGroup, &'r ModelMesh);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PassRule {
    Myth2,
    Tfl,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PassSkips {
    pub(crate) scene: bool,
    pub(crate) category4: bool,
    pub(crate) opaque: bool,
    pub(crate) sprites: bool,
}

pub(crate) struct WorldDraws<'r> {
    pub(crate) terrain_renderer: &'r TerrainRenderer,
    pub(crate) terrain_mesh: &'r TerrainMesh,
    pub(crate) landscape: Option<&'r wgpu::BindGroup>,
    pub(crate) model_renderer: &'r ModelRenderer,
    pub(crate) billboard_renderer: &'r BillboardRenderer,
    pub(crate) atlases: SpriteAtlases<'r>,
    pub(crate) billboard_buf: &'r wgpu::Buffer,
    pub(crate) billboards: &'r [BillboardInstance],
    pub(crate) sprite_order: &'r SpritePassOrder,
    pub(crate) category2: &'r TranslucentBatch,
    pub(crate) category2_draws: &'r [ModelDraw<'r>],
    pub(crate) reflections: &'r ReflectionBatch,
    pub(crate) reflection_draws: &'r [ModelDraw<'r>],
    pub(crate) soft_edge: Option<&'r ReflectionEdge>,
    pub(crate) water: Option<&'r mandrake_render::water::WaterShading>,
    pub(crate) opaque_models: &'r [ModelDraw<'r>],
    pub(crate) translucent: &'r TranslucentBatch,
    pub(crate) translucent_draws: &'r [ModelDraw<'r>],
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WorldPasses {
    pub(crate) rule: PassRule,
    pub(crate) clear: wgpu::Color,
    pub(crate) skip: PassSkips,
}

impl WorldPasses {
    pub(crate) fn order(&self, water: bool) -> PassRule {
        if water { PassRule::Myth2 } else { self.rule }
    }
}

pub(crate) fn finish_sprites(
    billboards: &mut Vec<BillboardInstance>,
    camera: &mandrake_view::camera::Camera,
) -> SpritePassOrder {
    let order = order_for_sprite_pass(billboards, camera.eye_position(), camera.view_axes().2);
    place_sprites_in_engine_view(billboards, camera);
    order
}

pub(crate) fn encode_world(
    encoder: &mut wgpu::CommandEncoder,
    targets: &WorldTargets<'_>,
    draws: &WorldDraws<'_>,
    passes: WorldPasses,
    timer: Option<&GpuTimer>,
) {
    let mark_pass = |pass: &mut wgpu::RenderPass<'_>, mark: Mark| {
        if let Some(timer) = timer {
            timer.mark_in_pass(pass, mark);
        }
    };
    let mark = |encoder: &mut wgpu::CommandEncoder, mark: Mark| {
        if let Some(timer) = timer {
            timer.mark(encoder, mark);
        }
    };
    let scene = &draws.terrain_renderer.uniform_bind_group;
    let rule = passes.order(draws.water.is_some());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Scene Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: targets.color,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(scene_clear(
                        passes.clear,
                        draws.terrain_renderer.config.target_format,
                    )),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: targets.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        if !passes.skip.scene {
            draws.model_renderer.render_category2(
                &mut pass,
                scene,
                draws.category2,
                draws.category2_draws,
            );
            mark_pass(&mut pass, Mark::Category2);
            match draws.water {
                Some(water) => water.render_bed(
                    &mut pass,
                    draws.terrain_renderer,
                    draws.terrain_mesh,
                    draws.landscape,
                ),
                None => draws.terrain_renderer.render_terrain_media(
                    &mut pass,
                    draws.terrain_mesh,
                    draws.landscape,
                ),
            }
        }
        mark_pass(&mut pass, Mark::Media);
    }
    if !passes.skip.category4 {
        encode_category4(encoder, targets, draws, rule, |encoder| {
            mark(encoder, Mark::SoftEdgeMask);
        });
    }
    if let (Some(water), PassRule::Myth2, false) = (draws.water, rule, passes.skip.scene) {
        let mut pass = begin_opaque_continue(encoder, targets.interface, targets.depth);
        water.render_under(
            &mut pass,
            draws.terrain_renderer,
            draws.terrain_mesh,
            draws.landscape,
        );
    }
    mark(encoder, Mark::Category4);
    {
        let mut pass = match rule {
            PassRule::Myth2 => begin_opaque_continue(encoder, targets.color, targets.depth),
            PassRule::Tfl => begin_depth_cleared(encoder, targets.color, targets.depth),
        };
        let water = match (draws.water, rule, passes.skip.scene) {
            (Some(water), PassRule::Myth2, false) if water.lowered_buffer().is_some() => {
                Some(water)
            }
            _ => None,
        };
        if let Some(water) = water {
            if !passes.skip.opaque {
                water.render_dry(
                    &mut pass,
                    draws.terrain_renderer,
                    draws.terrain_mesh,
                    draws.landscape,
                );
                water.render_wet(
                    &mut pass,
                    draws.terrain_renderer,
                    draws.terrain_mesh,
                    draws.landscape,
                );
            }
            water.render_surface_depth(
                &mut pass,
                draws.terrain_renderer,
                draws.terrain_mesh,
                draws.landscape,
            );
        }
        if !passes.skip.opaque {
            for (bind_group, mesh) in draws.opaque_models {
                draws
                    .model_renderer
                    .render(&mut pass, scene, None, Some(bind_group), mesh);
            }
        }
        mark_pass(&mut pass, Mark::OpaqueModels);
        if !passes.skip.opaque && water.is_none() {
            draws.terrain_renderer.render_terrain_dry(
                &mut pass,
                draws.terrain_mesh,
                draws.landscape,
            );
        }
        mark_pass(&mut pass, Mark::DryLandscape);
    }
    if !passes.skip.sprites {
        encode_sprite_category(encoder, targets, draws, |pass| {
            mark_pass(pass, Mark::SpriteOccluders);
        });
    }
    mark(encoder, Mark::Sprites);
}

pub(crate) fn scene_clear(linear: wgpu::Color, format: wgpu::TextureFormat) -> wgpu::Color {
    if format.is_srgb() {
        return linear;
    }
    let encode = |c: f64| {
        let c = c.clamp(0.0, 1.0);
        if c <= 0.0031308 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        }
    };
    wgpu::Color {
        r: encode(linear.r),
        g: encode(linear.g),
        b: encode(linear.b),
        a: linear.a,
    }
}

fn begin_depth_cleared<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    view: &'a wgpu::TextureView,
    depth: &'a wgpu::TextureView,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Third Pass (depth cleared)"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            depth_slice: None,
            view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        occlusion_query_set: None,
        timestamp_writes: None,
        multiview_mask: None,
    })
}

fn encode_sprite_category(
    encoder: &mut wgpu::CommandEncoder,
    targets: &WorldTargets<'_>,
    draws: &WorldDraws<'_>,
    after_occluders: impl FnOnce(&mut wgpu::RenderPass<'_>),
) {
    let (instances, translucent_batch) = (draws.billboards, draws.translucent);
    let scene_bind_group = &draws.terrain_renderer.uniform_bind_group;
    let models = draws.model_renderer;
    let steps = sprite_category_steps(instances, draws.sprite_order, translucent_batch.keys());
    let steps = match translucent_batch.view_proj() {
        Some(view_proj) => merge_sprite_steps(steps, instances, view_proj, |range| {
            translucent_batch.screen_bounds(range)
        }),
        None => steps,
    };
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Sprite Occluder Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: targets.color,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: targets.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        for (bind_group, mesh) in draws.opaque_models {
            models.render_depth_only(&mut pass, scene_bind_group, None, Some(bind_group), mesh);
        }
        draws
            .terrain_renderer
            .render_terrain_occluders(&mut pass, draws.terrain_mesh);
        after_occluders(&mut pass);
    }
    if steps.is_empty() {
        return;
    }
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Sprite Display Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            depth_slice: None,
            view: targets.interface,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: targets.depth,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        occlusion_query_set: None,
        timestamp_writes: None,
        multiview_mask: None,
    });
    let mut triangle_state = TrianglePassState::default();
    for step in &steps {
        match step {
            SpriteStep::Triangles(range) => models.render_sprite_triangles(
                &mut pass,
                scene_bind_group,
                translucent_batch,
                draws.translucent_draws,
                range.clone(),
                &mut triangle_state,
            ),
            SpriteStep::Billboards(range) if !range.is_empty() => {
                draws
                    .atlases
                    .for_each_run(instances, range.clone(), |atlas, run| {
                        draws.billboard_renderer.render_range(
                            &mut pass,
                            triangle_state.scene_to_bind(scene_bind_group),
                            Some(atlas),
                            draws.billboard_buf,
                            run,
                        );
                    });
            }
            SpriteStep::Interface(range) if !range.is_empty() => {
                draws.billboard_renderer.render_interface_range(
                    &mut pass,
                    triangle_state.scene_to_bind(scene_bind_group),
                    draws.billboard_buf,
                    range.clone(),
                )
            }
            SpriteStep::Billboards(_) | SpriteStep::Interface(_) => {}
        }
    }
}

fn encode_category4(
    encoder: &mut wgpu::CommandEncoder,
    targets: &WorldTargets<'_>,
    draws: &WorldDraws<'_>,
    rule: PassRule,
    after_mask: impl FnOnce(&mut wgpu::CommandEncoder),
) {
    let uniform_bind_group = &draws.terrain_renderer.uniform_bind_group;
    let steps = category4_steps(draws.sprite_order, draws.reflections.keys());
    if !steps.is_empty()
        && let Some(edge) = draws.soft_edge
    {
        let mask = match rule {
            PassRule::Myth2 => ReflectionEdge::encode_mask,
            PassRule::Tfl => ReflectionEdge::encode_mask_depth_cleared,
        };
        mask(
            edge,
            encoder,
            uniform_bind_group,
            draws.terrain_mesh,
            targets.depth,
        );
    }
    after_mask(encoder);
    let layer = draws.water.and_then(|w| w.reflection_layer());
    if steps.is_empty() && layer.is_none() {
        return;
    }
    let edge = draws.soft_edge.map(ReflectionEdge::bind_group);
    let mut pass = match layer {
        Some(view) => begin_cleared_layer(encoder, view, targets.depth),
        None => begin_opaque_continue(encoder, targets.interface, targets.depth),
    };
    for step in steps.iter().cloned() {
        match step {
            Category4Step::Billboards(range) => {
                draws
                    .atlases
                    .for_each_run(draws.billboards, range, |atlas, run| {
                        let split = draws.water.is_some_and(|w| {
                            w.render_category4_billboards(
                                &mut pass,
                                draws.billboard_renderer,
                                uniform_bind_group,
                                Some(atlas),
                                draws.billboard_buf,
                                run.clone(),
                                edge,
                                false,
                            )
                        });
                        if !split {
                            draws.billboard_renderer.render_reflections(
                                &mut pass,
                                uniform_bind_group,
                                Some(atlas),
                                draws.billboard_buf,
                                run,
                                edge,
                            );
                        }
                    });
            }
            Category4Step::Triangles(range) => draws.model_renderer.render_reflection_triangles(
                &mut pass,
                uniform_bind_group,
                draws.reflections,
                draws.reflection_draws,
                range,
                edge,
            ),
        }
    }
    drop(pass);
    if let Some((water, (color, height))) = draws
        .water
        .and_then(|w| w.submerged_layer().map(|views| (w, views)))
    {
        let mut pass = begin_submerged_layer(encoder, color, height, targets.depth);
        for step in steps {
            if let Category4Step::Billboards(range) = step {
                draws
                    .atlases
                    .for_each_run(draws.billboards, range, |atlas, run| {
                        water.render_category4_billboards(
                            &mut pass,
                            draws.billboard_renderer,
                            uniform_bind_group,
                            Some(atlas),
                            draws.billboard_buf,
                            run,
                            None,
                            true,
                        );
                    });
            }
        }
    }
}

fn begin_submerged_layer<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    color: &'a wgpu::TextureView,
    height: &'a wgpu::TextureView,
    depth_view: &'a wgpu::TextureView,
) -> wgpu::RenderPass<'a> {
    let attachment = |view| {
        Some(wgpu::RenderPassColorAttachment {
            depth_slice: None,
            view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })
    };
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Water Submerged Layer Pass"),
        color_attachments: &[attachment(color), attachment(height)],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth_view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        occlusion_query_set: None,
        timestamp_writes: None,
        multiview_mask: None,
    })
}

fn begin_cleared_layer<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    view: &'a wgpu::TextureView,
    depth_view: &'a wgpu::TextureView,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Water Reflection Layer Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            depth_slice: None,
            view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth_view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        occlusion_query_set: None,
        timestamp_writes: None,
        multiview_mask: None,
    })
}

pub(crate) fn begin_opaque_continue<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    view: &'a wgpu::TextureView,
    depth_view: &'a wgpu::TextureView,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Opaque Scene Pass (continued)"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            depth_slice: None,
            view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth_view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        occlusion_query_set: None,
        timestamp_writes: None,
        multiview_mask: None,
    })
}

pub(crate) struct BarTargets<'a> {
    pub(crate) device: &'a wgpu::Device,
    pub(crate) queue: &'a wgpu::Queue,
    pub(crate) overlay: &'a mut mandrake_render::OverlayRenderer,
    pub(crate) scaler: &'a mut mandrake_render::InterfaceScaler,
    pub(crate) view: &'a wgpu::TextureView,
    pub(crate) window: (u32, u32),
}

pub(crate) fn encode_bars(
    to: BarTargets<'_>,
    encoder: &mut wgpu::CommandEncoder,
    (ui, filter): (
        &crate::ui_scale::UiScale,
        mandrake_view::interface_picture::InterfaceFilter,
    ),
    draws: &[mandrake_render::OverlayDraw<'_>],
    dialog: &mut Option<&mut dyn mandrake_render::DrawInterface>,
) {
    let BarTargets {
        device,
        queue,
        overlay,
        scaler,
        view,
        window,
    } = to;
    if !(ui.is_scaled() && ui.window == window) {
        overlay.encode(device, queue, encoder, view, window, draws);
        return;
    }
    let layer = scaler.layer_view(device, ui.logical);
    overlay.encode_cleared(device, queue, encoder, layer, ui.logical, draws);
    let dialog = dialog.take();
    let had_dialog = dialog.is_some();
    if let Some(dialog) = dialog {
        dialog.draw(&mut mandrake_render::InterfaceFrame {
            device,
            queue,
            encoder,
            target: layer,
            format: overlay.format,
            size: ui.logical,
        });
    }
    if had_dialog {
        scaler.composite(queue, encoder, view, window, filter);
    } else {
        let content: Vec<[i32; 4]> = draws.iter().map(|d| d.dst).collect();
        scaler.composite_content(queue, encoder, view, window, filter, &content);
    }
}

#[cfg(test)]
pub(crate) fn encode_interface_pass(
    encoder: &mut wgpu::CommandEncoder,
    interface_view: &wgpu::TextureView,
    depth_view: &wgpu::TextureView,
    billboard_renderer: &BillboardRenderer,
    uniform_bind_group: &wgpu::BindGroup,
    billboard_buf: &wgpu::Buffer,
    ranges: &[core::ops::Range<u32>],
) {
    if ranges.iter().all(|r| r.is_empty()) {
        return;
    }
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Interface Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            depth_slice: None,
            view: interface_view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth_view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        occlusion_query_set: None,
        timestamp_writes: None,
        multiview_mask: None,
    });
    for (i, range) in ranges.iter().filter(|r| !r.is_empty()).enumerate() {
        billboard_renderer.render_interface_range(
            &mut pass,
            (i == 0).then_some(uniform_bind_group),
            billboard_buf,
            range.clone(),
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum WorldFiltering {
    Myth2 {
        sampling: Sampling,
        sprite_filter: Option<UiFilter>,
    },
}

impl WorldFiltering {
    pub(crate) fn tfl(sampling: Sampling, sprite_filter: Option<UiFilter>) -> Self {
        Self::Myth2 {
            sampling,
            sprite_filter,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SceneLighting<'a> {
    pub(crate) ambient_tint: [f32; 3],
    pub(crate) enabled: bool,
    pub(crate) lights: &'a [mandrake_render::PointLight],
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WorldLook<'a> {
    pub(crate) filtering: WorldFiltering,
    pub(crate) fog: Option<(&'a MapFog, f32)>,
    pub(crate) lighting: Option<SceneLighting<'a>>,
}

pub(crate) fn world_uniforms(
    camera: &mandrake_view::camera::Camera,
    (width, height): (u32, u32),
    look: &WorldLook<'_>,
) -> SceneUniforms {
    let uniforms = SceneUniforms::from_camera(camera, width as f32 / height as f32)
        .with_viewport(width, height);
    let WorldFiltering::Myth2 {
        sampling,
        sprite_filter,
    } = look.filtering;
    let uniforms = crate::ui_scale::with_world_sampling(uniforms, sampling, sprite_filter);
    let uniforms = match look.fog {
        Some((fog, seconds)) => fog.apply(uniforms, seconds),
        None => uniforms,
    };
    match look.lighting {
        Some(l) => uniforms.with_lighting(l.ambient_tint, l.enabled, l.lights),
        None => uniforms,
    }
}

pub(crate) fn create_depth_view(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> wgpu::TextureView {
    let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Depth Texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    depth_texture.create_view(&wgpu::TextureViewDescriptor::default())
}

#[cfg(test)]
mod team_cache_tests {
    use super::*;

    #[test]
    fn a_range_splits_by_texture_in_order() {
        let mut instances = vec![BillboardInstance::default(); 6];
        instances[2].flags |= TEAM_CACHE_FLAG;
        instances[3].flags |= TEAM_CACHE_FLAG;
        instances[5].flags |= TEAM_CACHE_FLAG;
        assert_eq!(
            team_cache_runs(&instances, 1..6),
            vec![(false, 1..2), (true, 2..4), (false, 4..5), (true, 5..6)]
        );
        assert_eq!(team_cache_runs(&instances, 0..2), vec![(false, 0..2)]);
    }
}
