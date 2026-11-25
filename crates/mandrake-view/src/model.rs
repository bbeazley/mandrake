use crate::camera::Mat4;
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub struct ModelVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub alpha: f32,
}

impl ModelVertex {
    pub const fn new(position: [f32; 3], normal: [f32; 3], uv: [f32; 2]) -> Self {
        Self {
            position,
            normal,
            uv,
            alpha: 1.0,
        }
    }

    pub const fn with_alpha(mut self, alpha: f32) -> Self {
        self.alpha = alpha;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TranslucentRun {
    pub instance: usize,
    pub submesh: usize,
    pub indices: core::ops::Range<u32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TranslucentDrawList {
    pub indices: Vec<u32>,
    pub runs: Vec<TranslucentRun>,
    pub keys: Vec<f32>,
    pub bounds: Vec<Option<[f32; 4]>>,
    pub view_proj: Option<Mat4>,
}
