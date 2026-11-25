pub struct InterfaceFrame<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub target: &'a wgpu::TextureView,
    pub format: wgpu::TextureFormat,
    pub size: (u32, u32),
}

pub trait DrawInterface {
    fn draw(&mut self, frame: &mut InterfaceFrame<'_>);
}
