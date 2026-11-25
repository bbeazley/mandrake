use crate::ui_scale::UiScale;
use mandrake_render::{InterfaceScaler, OverlayDraw, OverlayImage, OverlayLayer, OverlayRenderer};
use mandrake_tag::Rect;
use mandrake_tag::font::Bitmap32;
use mandrake_view::interface_bitmap::InterfaceArt;
use mandrake_view::interface_picture::InterfaceFilter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CursorBitmap {
    pub rgba: Vec<u8>,
    pub size: (u32, u32),
    pub hot: (i32, i32),
}

impl CursorBitmap {
    pub(crate) fn of_art(art: &InterfaceArt, sequence: i32) -> Option<Self> {
        let (bitmap, instance) = art.bitmap(sequence, 0, 0)?;
        let (w, h) = (bitmap.width, bitmap.height);
        if w <= 0 || h <= 0 {
            return None;
        }
        let mut canvas = Bitmap32::new(w, h, 0);
        mandrake_view::interface_bitmap::copy_bitmap(
            &mut canvas,
            bitmap,
            art.palette(),
            0,
            0,
            Rect::new(0, 0, h, w),
            0,
        );
        Some(Self {
            rgba: canvas
                .pixels
                .iter()
                .flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8])
                .collect(),
            size: (w as u32, h as u32),
            hot: (i32::from(instance.origin.0), i32::from(instance.origin.1)),
        })
    }

    pub(crate) const fn top_left(&self, at: (i32, i32)) -> (i32, i32) {
        (at.0 - self.hot.0, at.1 - self.hot.1)
    }
}

pub(crate) struct SoftwareCursor {
    bitmap: CursorBitmap,
    gpu: Option<(OverlayRenderer, OverlayImage)>,
    scaler: Option<InterfaceScaler>,
}

impl SoftwareCursor {
    pub(crate) const fn new(bitmap: CursorBitmap) -> Self {
        Self {
            bitmap,
            gpu: None,
            scaler: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        ui: &UiScale,
        at: (i32, i32),
        filter: InterfaceFilter,
    ) {
        let b = &self.bitmap;
        let (overlay, image) = self.gpu.get_or_insert_with(|| {
            let overlay = OverlayRenderer::new(device, format);
            let image = overlay.create_image(device, queue, b.size.0, b.size.1, &b.rgba);
            (overlay, image)
        });
        if !ui.is_scaled() {
            let (x, y) = b.top_left(at);
            let draw = OverlayDraw::at(OverlayLayer::Dialog, image, x, y, 0xff);
            overlay.encode(device, queue, encoder, target, ui.window, &[draw]);
            return;
        }
        let (x, y) = b.top_left(ui.to_logical(at.0 as f32, at.1 as f32));
        let draw = OverlayDraw::at(OverlayLayer::Dialog, image, x, y, 0xff);
        let scaler = self
            .scaler
            .get_or_insert_with(|| InterfaceScaler::new(device, format));
        let layer = scaler.layer_view(device, ui.logical);
        overlay.encode_cleared(device, queue, encoder, layer, ui.logical, &[draw]);
        scaler.composite_content(queue, encoder, target, ui.window, filter, &[draw.dst]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hot_spot_is_at_the_mouse() {
        let b = CursorBitmap {
            rgba: vec![0; 4],
            size: (1, 1),
            hot: (3, 5),
        };
        assert_eq!(b.top_left((100, 200)), (97, 195));
    }
}
