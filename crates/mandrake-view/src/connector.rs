use crate::billboard::{AtlasFrame, BILLBOARD_FLAG_CONNECTOR, BillboardInstance};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConnectorStrip {
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
    pub below_attach: f32,
    pub height: f32,
}

pub fn instance_rect_vertical(key_y: i16, height: u16, scale: i32) -> (i16, i16) {
    let top = scale.wrapping_mul(i32::from(key_y)) >> 16;
    let below = (i32::from(height) - i32::from(key_y)).wrapping_mul(scale) >> 16;
    (top as i16, (below as i16).wrapping_neg())
}

impl ConnectorStrip {
    pub fn from_atlas_frame(frame: &AtlasFrame, rect: (i16, i16), world_one: f32) -> Self {
        let (top, bottom) = (f32::from(rect.0), f32::from(rect.1));
        Self {
            uv_min: frame.uv_min,
            uv_max: frame.uv_max,
            below_attach: top / world_one,
            height: (top + bottom) / world_one,
        }
    }
}

pub fn connector_chain_spans(points: &[Option<[f32; 3]>]) -> Vec<(usize, usize)> {
    points
        .windows(2)
        .enumerate()
        .filter_map(|(i, w)| match (w[0], w[1]) {
            (Some(a), Some(b)) if a != b => Some((i, i + 1)),
            _ => None,
        })
        .collect()
}

pub fn connector_span_instances(
    p0: [f32; 3],
    p1: [f32; 3],
    segment_count: usize,
    strip: &ConnectorStrip,
) -> Vec<BillboardInstance> {
    let n = segment_count.max(1);
    let lerp = |t: f32| {
        [
            p0[0] + (p1[0] - p0[0]) * t,
            p0[1] + (p1[1] - p0[1]) * t,
            p0[2] + (p1[2] - p0[2]) * t,
        ]
    };
    (0..n)
        .map(|i| {
            let a = lerp(i as f32 / n as f32);
            let b = lerp((i + 1) as f32 / n as f32);
            BillboardInstance {
                position: [a[0], a[1] - strip.below_attach, a[2]],
                flags: BILLBOARD_FLAG_CONNECTOR,
                size: [b[1] - a[1], strip.height],
                uv_min: strip.uv_min,
                uv_max: strip.uv_max,
                tint: [1.0, 1.0, 1.0, 1.0],
                anchor: [b[0] - a[0], b[2] - a[2]],
                ..BillboardInstance::default()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip() -> ConnectorStrip {
        let frame = AtlasFrame {
            index: 0,
            uv_min: [0.0, 0.0],
            uv_max: [0.5, 0.25],
            width: 64,
            height: 32,
            offset_x: 20,
            offset_y: 36,
        };
        ConnectorStrip::from_atlas_frame(&frame, instance_rect_vertical(36, 32, 0xa_0000), 512.0)
    }

    #[test]
    fn test_strip_extents_from_instance_rect() {
        assert_eq!(instance_rect_vertical(36, 32, 0xa_0000), (360, 40));
        assert_eq!(instance_rect_vertical(7, 15, 0xa_0000), (70, -80));
        let s = strip();
        assert!((s.below_attach - 360.0 / 512.0).abs() < 1e-6);
        assert!((s.height - 400.0 / 512.0).abs() < 1e-6);
        assert!(((s.height - s.below_attach) - 40.0 / 512.0).abs() < 1e-6);
    }

    #[test]
    fn test_chain_spans_break_on_missing_points() {
        let p = |x: f32| Some([x, 0.0, 0.0]);
        let spans = connector_chain_spans(&[p(0.0), p(1.0), None, p(3.0), p(4.0)]);
        assert_eq!(spans, vec![(0, 1), (3, 4)]);
        assert!(connector_chain_spans(&[p(0.0)]).is_empty());
    }

    #[test]
    fn test_closed_loop_chain_by_repeated_id() {
        let a = Some([0.0, 0.0, 0.0]);
        let b = Some([2.0, 0.0, 0.0]);
        let c = Some([2.0, 0.0, 2.0]);
        assert_eq!(
            connector_chain_spans(&[a, b, c, a]),
            vec![(0, 1), (1, 2), (2, 3)]
        );
        assert!(connector_chain_spans(&[a, a]).is_empty());
    }

    #[test]
    fn test_span_instances_tile_span_exactly() {
        let s = strip();
        let p0 = [0.0, 1.0, 0.0];
        let p1 = [3.0, 1.6, 4.0];
        let quads = connector_span_instances(p0, p1, 5, &s);
        assert_eq!(quads.len(), 5);
        let mut end = [0.0f32; 3];
        for (i, q) in quads.iter().enumerate() {
            assert_eq!(q.flags & BILLBOARD_FLAG_CONNECTOR, BILLBOARD_FLAG_CONNECTOR);
            let start = [q.position[0], q.position[1] + s.below_attach, q.position[2]];
            if i > 0 {
                for k in 0..3 {
                    assert!(
                        (start[k] - end[k]).abs() < 1e-5,
                        "segments must be contiguous"
                    );
                }
            }
            end = [
                start[0] + q.anchor[0],
                start[1] + q.size[0],
                start[2] + q.anchor[1],
            ];
            assert!((q.size[1] - s.height).abs() < 1e-6);
            assert_eq!(q.uv_min, s.uv_min);
            assert_eq!(q.uv_max, s.uv_max);
        }
        for k in 0..3 {
            assert!((end[k] - p1[k]).abs() < 1e-5);
        }
        assert!((quads[0].position[1] - (1.0 - s.below_attach)).abs() < 1e-6);
    }

    #[test]
    fn test_zero_segment_count_clamps_to_one() {
        let quads = connector_span_instances([0.0; 3], [0.5, 0.0, 0.0], 0, &strip());
        assert_eq!(quads.len(), 1);
    }
}
