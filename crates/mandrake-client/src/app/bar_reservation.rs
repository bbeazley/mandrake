#![cfg_attr(any(target_arch = "wasm32", target_vendor = "apple"), allow(dead_code))]

const MIB: u64 = 1 << 20;
const HOST_BLOCK_MIN: u64 = 64 * MIB;
const LEFT_FREE: u64 = 32 * MIB;
const STEP: u64 = 4 * MIB;
const SMALL_BAR_MAX: u64 = 1024 * MIB;

const DEVICE_LOCAL: u32 = 0x1;
const HOST_VISIBLE: u32 = 0x2;
const HOST_COHERENT: u32 = 0x4;
const BAR_FLAGS: u32 = DEVICE_LOCAL | HOST_VISIBLE | HOST_COHERENT;

pub(crate) struct BarReservation {
    pub(crate) size: u64,
    pub(crate) heap_size: u64,
    release: Option<Box<dyn FnOnce() + Send>>,
}

impl Drop for BarReservation {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

#[cfg(not(any(target_arch = "wasm32", target_vendor = "apple")))]
fn call_with<T: Default, R>(call: impl FnOnce(&T) -> R, build: impl FnOnce(T) -> T) -> R {
    call(&build(T::default()))
}

impl BarReservation {
    #[cfg(not(any(target_arch = "wasm32", target_vendor = "apple")))]
    pub(crate) fn take(device: &wgpu::Device) -> Option<Self> {
        let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }?;
        let raw = hal.raw_device();
        let props = unsafe {
            hal.shared_instance()
                .raw_instance()
                .get_physical_device_memory_properties(hal.raw_physical_device())
        };
        let (type_index, memory_type) = props.memory_types[..props.memory_type_count as usize]
            .iter()
            .enumerate()
            .find(|(_, t)| t.property_flags.as_raw() & BAR_FLAGS == BAR_FLAGS)?;
        let heap_size = props.memory_heaps[memory_type.heap_index as usize].size;
        if heap_size > SMALL_BAR_MAX {
            return None;
        }
        let type_index = type_index as u32;
        let allocate = |size: u64| {
            call_with(
                |info| unsafe { raw.allocate_memory(info, None) },
                |info| info.allocation_size(size).memory_type_index(type_index),
            )
            .ok()
        };
        let mut free = heap_size / STEP * STEP;
        let found = loop {
            if free < HOST_BLOCK_MIN {
                return None;
            }
            if let Some(memory) = allocate(free) {
                break memory;
            }
            free -= STEP;
        };
        unsafe { raw.free_memory(found, None) };
        let size = free - LEFT_FREE;
        let memory = allocate(size)?;
        let device = device.clone();
        Some(Self {
            size,
            heap_size,
            release: Some(Box::new(move || {
                if let Some(hal) = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() } {
                    unsafe { hal.raw_device().free_memory(memory, None) };
                }
            })),
        })
    }

    #[cfg(any(target_arch = "wasm32", target_vendor = "apple"))]
    pub(crate) fn take(_device: &wgpu::Device) -> Option<Self> {
        None
    }
}
