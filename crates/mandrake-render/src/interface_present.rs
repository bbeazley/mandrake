use mandrake_view::canvas_fit::CanvasFit;
use mandrake_view::interface_picture::{
    EDGE_ROWS, InterfaceFilter, InterfaceLayer, InterfacePicture, MenuCanvas, TEXTURE_PAD_ROWS,
};

use crate::interface_frame::InterfaceFrame;
use crate::interface_scale::{InterfaceScaler, ScrollDraw};
use crate::overlay::{OverlayDraw, OverlayImage, OverlayLayer, OverlayRenderer};

pub struct MenuCanvasGpu {
    overlay: OverlayRenderer,
    scaler: InterfaceScaler,
    image: Option<OverlayImage>,
    revision: u64,
    prologue_generation: u64,
}

impl MenuCanvasGpu {
    pub fn new(frame: &InterfaceFrame<'_>) -> Self {
        Self {
            overlay: OverlayRenderer::new(frame.device, frame.format),
            scaler: InterfaceScaler::new(frame.device, frame.format),
            image: None,
            revision: u64::MAX,
            prologue_generation: 0,
        }
    }

    pub fn draw(
        &mut self,
        frame: &mut InterfaceFrame<'_>,
        menu: MenuCanvas<'_>,
        filter: InterfaceFilter,
    ) {
        let changed = self.revision != menu.revision;
        if changed {
            let (w, h) = (menu.canvas.width as u32, menu.canvas.height as u32);
            let rgba = menu.canvas.to_rgba();
            match &self.image {
                Some(image) if (image.width, image.height) == (w, h) => {
                    OverlayRenderer::update_image(frame.queue, image, &rgba);
                }
                _ => {
                    self.image =
                        Some(
                            self.overlay
                                .create_image(frame.device, frame.queue, w, h, &rgba),
                        );
                }
            }
            self.revision = menu.revision;
        }
        let canvas = menu.size;
        if let (true, Some(image)) = (changed, self.image.as_ref()) {
            let draw = OverlayDraw::at(OverlayLayer::Dialog, image, 0, 0, 0xff);
            let layer = self.scaler.layer_view(frame.device, canvas);
            self.overlay.encode_cleared(
                frame.device,
                frame.queue,
                frame.encoder,
                layer,
                canvas,
                &[draw],
            );
        }
        let fit = CanvasFit::new(frame.size, canvas);
        self.scaler.composite_into(
            frame.queue,
            frame.encoder,
            frame.target,
            Some([fit.origin.0, fit.origin.1, fit.size.0, fit.size.1]),
            frame.size,
            filter,
        );
        if let Some(prol) = menu.prologue
            && self.prologue_generation != prol.generation
        {
            let (w, h, rgba) = prol.to_padded_premultiplied_rgba(TEXTURE_PAD_ROWS as u32);
            self.scaler
                .set_scroll_layer(frame.device, frame.queue, w, h, &rgba);
            self.prologue_generation = prol.generation;
        }
        if let Some((rect, scroll, strength, true)) = menu.scroll {
            let scale_x = fit.size.0 as f32 / canvas.0.max(1) as f32;
            let scale_y = fit.size.1 as f32 / canvas.1.max(1) as f32;
            let draw = ScrollDraw {
                dst: [
                    fit.origin.0 as f32 + rect.left as f32 * scale_x,
                    fit.origin.1 as f32 + rect.top as f32 * scale_y,
                    rect.width() as f32 * scale_x,
                    rect.height() as f32 * scale_y,
                ],
                view_size: [rect.width() as f32, rect.height() as f32],
                uv_offset: [0.0, scroll + TEXTURE_PAD_ROWS as f32],
                fade_rows: EDGE_ROWS as f32,
                alpha: (strength as f32 / 255.0).clamp(0.0, 1.0),
            };
            self.scaler.composite_scroll(
                frame.queue,
                frame.encoder,
                frame.target,
                frame.size,
                draw,
                filter,
            );
        }
    }
}

pub struct InterfacePresenter {
    menu: MenuCanvasGpu,
    overlay: OverlayRenderer,
    scaler: InterfaceScaler,
    images: Vec<OverlayImage>,
    revision: u64,
}

impl InterfacePresenter {
    pub fn new(frame: &InterfaceFrame<'_>) -> Self {
        Self {
            menu: MenuCanvasGpu::new(frame),
            overlay: OverlayRenderer::new(frame.device, frame.format),
            scaler: InterfaceScaler::new(frame.device, frame.format),
            images: Vec::new(),
            revision: u64::MAX,
        }
    }

    pub fn draw(
        &mut self,
        frame: &mut InterfaceFrame<'_>,
        picture: InterfacePicture<'_>,
        filter: InterfaceFilter,
    ) {
        match picture {
            InterfacePicture::Menu(menu) => self.menu.draw(frame, menu, filter),
            InterfacePicture::Layers { layers, revision } => {
                self.draw_layers(frame, &layers, revision, filter);
            }
        }
    }

    fn draw_layers(
        &mut self,
        frame: &mut InterfaceFrame<'_>,
        layers: &[InterfaceLayer<'_>],
        revision: u64,
        filter: InterfaceFilter,
    ) {
        if self.revision != revision {
            self.images.truncate(layers.len());
            for (i, layer) in layers.iter().enumerate() {
                let (w, h) = (layer.bitmap.width as u32, layer.bitmap.height as u32);
                let rgba = layer.bitmap.to_rgba();
                match self.images.get(i) {
                    Some(image) if (image.width, image.height) == (w, h) => {
                        OverlayRenderer::update_image(frame.queue, image, &rgba);
                    }
                    _ => {
                        let image =
                            self.overlay
                                .create_image(frame.device, frame.queue, w, h, &rgba);
                        if i < self.images.len() {
                            self.images[i] = image;
                        } else {
                            self.images.push(image);
                        }
                    }
                }
            }
            self.revision = revision;
        }
        let mut draws: Vec<OverlayDraw<'_>> = Vec::new();
        for (layer, image) in layers.iter().zip(&self.images) {
            let (w, h) = (layer.bitmap.width, layer.bitmap.height);
            if w <= 0 || h <= 0 {
                continue;
            }
            if layer.dst[2] != w || layer.dst[3] != h {
                if !draws.is_empty() {
                    self.overlay.encode(
                        frame.device,
                        frame.queue,
                        frame.encoder,
                        frame.target,
                        frame.size,
                        &draws,
                    );
                    draws.clear();
                }
                let src_size = (w as u32, h as u32);
                let draw = OverlayDraw::at(OverlayLayer::Dialog, image, 0, 0, 0xff);
                let view = self.scaler.layer_view(frame.device, src_size);
                self.overlay.encode_cleared(
                    frame.device,
                    frame.queue,
                    frame.encoder,
                    view,
                    src_size,
                    &[draw],
                );
                self.scaler.composite_into(
                    frame.queue,
                    frame.encoder,
                    frame.target,
                    Some(layer.dst),
                    frame.size,
                    filter,
                );
            } else {
                let mut draw = OverlayDraw::at(OverlayLayer::Dialog, image, 0, 0, 0xff);
                draw.dst = layer.dst;
                draws.push(draw);
            }
        }
        if !draws.is_empty() {
            self.overlay.encode(
                frame.device,
                frame.queue,
                frame.encoder,
                frame.target,
                frame.size,
                &draws,
            );
        }
    }
}
