use crate::camera::Mat4;
use bytemuck::{Pod, Zeroable};

pub trait VertexLayout {
    fn desc() -> wgpu::VertexBufferLayout<'static>;
}

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub struct TerrainVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub cell_flags: u32,
}

impl TerrainVertex {
    pub const ATTRIBS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x2,
        2 => Uint32,
    ];

    pub const fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: core::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }

    pub const fn new(position: [f32; 3], uv: [f32; 2], cell_flags: u32) -> Self {
        Self {
            position,
            uv,
            cell_flags,
        }
    }
}

pub const OCCLUDER_DEPTH_POINTS: usize = 6;

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub struct OccluderVertex {
    pub position: [f32; 3],
    pub depth: [[f32; 3]; OCCLUDER_DEPTH_POINTS],
}

impl OccluderVertex {
    pub const ATTRIBS: [wgpu::VertexAttribute; 1 + OCCLUDER_DEPTH_POINTS] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x3,
        2 => Float32x3,
        3 => Float32x3,
        4 => Float32x3,
        5 => Float32x3,
        6 => Float32x3,
    ];

    pub const fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: core::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }

    pub const MESH_DEPTH_BIAS: f32 = 128.0 / 512.0;

    pub fn occluder_depth(depths: impl IntoIterator<Item = f32>) -> f32 {
        depths.into_iter().fold(f32::MIN, f32::max) + Self::MESH_DEPTH_BIAS
    }

    pub fn occluder_clip(&self, view_proj: &Mat4, eye: [f32; 3]) -> [f32; 4] {
        let clip = |p: [f32; 3]| {
            let m = &view_proj.cols;
            let mut o = [0.0f32; 4];
            for (r, v) in o.iter_mut().enumerate() {
                *v = m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r];
            }
            o
        };
        let c = clip(self.position);
        if c[3] <= 1e-4 {
            return c;
        }
        let d = Self::occluder_depth(self.depth.map(|p| clip(p)[3]));
        let eye_z = clip(eye)[2];
        let s = d / c[3];
        [c[0] * s, c[1] * s, c[2] * s + eye_z * (1.0 - s), d]
    }

    pub fn with_depth(corners: [[f32; 3]; 3], depth: &[[f32; 3]]) -> [Self; 3] {
        let last = depth.last().copied().unwrap_or(corners[0]);
        let depth = core::array::from_fn(|i| depth.get(i).copied().unwrap_or(last));
        corners.map(|position| Self { position, depth })
    }

    pub fn triangle(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [Self; 3] {
        Self::with_depth([a, b, c], &[a, b, c])
    }

    pub fn from_triangles(positions: &[[f32; 3]], indices: &[u32]) -> Vec<Self> {
        let mut out = Vec::with_capacity(indices.len());
        for tri in indices.as_chunks::<3>().0 {
            let p = |i: u32| positions.get(i as usize).copied().unwrap_or_default();
            out.extend(Self::triangle(p(tri[0]), p(tri[1]), p(tri[2])));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vertex_layout_stride_and_offsets() {
        assert_eq!(core::mem::size_of::<TerrainVertex>(), 24);
        let desc = TerrainVertex::desc();
        assert_eq!(desc.array_stride, 24);
        assert_eq!(desc.attributes.len(), 3);
        assert_eq!(desc.attributes[0].offset, 0);
        assert_eq!(desc.attributes[1].offset, 12);
        assert_eq!(desc.attributes[2].offset, 20);
    }
}
