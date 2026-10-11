#![allow(
    clippy::struct_field_names,
    clippy::too_many_lines,
    clippy::used_underscore_binding,
    clippy::wildcard_imports
)]

use super::*;
use std::mem::ManuallyDrop;

mod encode;
mod execution;

use encode::EncodeResources;
use execution::ExecutionResources;

const MAX_COMPUTE_SHADER_BYTES: usize = 64 * 1024;
const SPIRV_MAGIC: u32 = 0x0723_0203;
const VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO: i32 = 14;
const VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO: i32 = 15;
const VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO: i32 = 16;
const VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO: i32 = 31;
const VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO: i32 = 18;
const VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO: i32 = 29;
const VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO: i32 = 30;
const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO: i32 = 32;
const VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO: i32 = 33;
const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO: i32 = 34;
const VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET: i32 = 35;
const VK_IMAGE_TYPE_2D: i32 = 1;
const VK_IMAGE_VIEW_TYPE_2D: i32 = 1;
const VK_IMAGE_VIEW_TYPE_2D_ARRAY: i32 = 5;
const VK_IMAGE_TILING_OPTIMAL: i32 = 0;
const VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT: i32 = 1_000_158_000;
const VK_IMAGE_LAYOUT_UNDEFINED: i32 = 0;
const VK_IMAGE_LAYOUT_GENERAL: i32 = 1;
const VK_IMAGE_USAGE_TRANSFER_SRC_BIT: u32 = 0x1;
const VK_IMAGE_USAGE_TRANSFER_DST_BIT: u32 = 0x2;
const VK_IMAGE_USAGE_SAMPLED_BIT: u32 = 0x4;
const VK_IMAGE_USAGE_STORAGE_BIT: u32 = 0x8;
const VK_IMAGE_USAGE_VIDEO_ENCODE_DPB_BIT_KHR: u32 = 0x0000_8000;
const VK_IMAGE_ASPECT_COLOR_BIT: u32 = 0x1;
const VK_SAMPLE_COUNT_1_BIT: u32 = 0x1;
const VK_SHARING_MODE_CONCURRENT: i32 = 1;
const VK_FORMAT_R8_UNORM: i32 = 9;
const VK_FORMAT_R8G8_UNORM: i32 = 16;
const VK_FORMAT_R8G8B8A8_UNORM: i32 = 37;
const VK_FORMAT_B8G8R8A8_UNORM: i32 = 44;
const VK_COMPONENT_SWIZZLE_IDENTITY: i32 = 0;
const VK_DESCRIPTOR_TYPE_STORAGE_IMAGE: i32 = 3;
const VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER: i32 = 1;
const VK_DESCRIPTOR_TYPE_STORAGE_BUFFER: i32 = 7;
const VK_SHADER_STAGE_COMPUTE_BIT: u32 = 0x20;
const PUSH_CONSTANT_BYTES: u32 = 20;
// Four conversion/input slots keep the compute and video queues fed at
// 120 FPS on 1440p sources. The encoder still uses only two alternating DPB
// slots: video-queue submission order makes reusing a DPB slot after two
// pictures safe, while coupling conversion depth to DPB depth unnecessarily
// stalled the producer every other frame.
const CONVERSION_SLOT_COUNT: usize = redunar_capture::REPLAY_ENCODER_PIPELINE_DEPTH;
const DPB_SLOT_COUNT: usize = 2;
const REPLAY_CONVERSION_SPIRV: &[u8] = include_bytes!("../shaders/replay_rgba_to_nv12.comp.spv");
const REPLAY_KMS_CONVERSION_SPIRV: &[u8] =
    include_bytes!("../shaders/replay_kms_rgba_to_nv12.comp.spv");

type VkImage = u64;
type VkImageView = u64;
type VkDescriptorSetLayout = u64;
type VkDescriptorPool = u64;
type VkDescriptorSet = u64;
type VkPipelineLayout = u64;
type VkPipeline = u64;
type VkPipelineCache = u64;
type VkShaderModule = u64;
type VkSampler = u64;
type VkBufferView = u64;

#[repr(C)]
struct VkImageCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    image_type: i32,
    format: i32,
    extent: VkExtent3d,
    mip_levels: u32,
    array_layers: u32,
    samples: u32,
    tiling: i32,
    usage: u32,
    sharing_mode: i32,
    queue_family_index_count: u32,
    queue_family_indices: *const u32,
    initial_layout: i32,
}

#[repr(C)]
struct VkComponentMapping {
    r: i32,
    g: i32,
    b: i32,
    a: i32,
}

#[repr(C)]
struct VkImageSubresourceRange {
    aspect_mask: u32,
    base_mip_level: u32,
    level_count: u32,
    base_array_layer: u32,
    layer_count: u32,
}

#[repr(C)]
struct VkImageViewCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    image: VkImage,
    view_type: i32,
    format: i32,
    components: VkComponentMapping,
    subresource_range: VkImageSubresourceRange,
}

#[repr(C)]
struct VkDescriptorSetLayoutBinding {
    binding: u32,
    descriptor_type: i32,
    descriptor_count: u32,
    stage_flags: u32,
    immutable_samplers: *const VkSampler,
}

#[repr(C)]
struct VkDescriptorSetLayoutCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    binding_count: u32,
    bindings: *const VkDescriptorSetLayoutBinding,
}

#[repr(C)]
struct VkDescriptorPoolSize {
    descriptor_type: i32,
    descriptor_count: u32,
}

#[repr(C)]
struct VkDescriptorPoolCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    max_sets: u32,
    pool_size_count: u32,
    pool_sizes: *const VkDescriptorPoolSize,
}

#[repr(C)]
struct VkDescriptorSetAllocateInfo {
    s_type: i32,
    p_next: *const c_void,
    descriptor_pool: VkDescriptorPool,
    descriptor_set_count: u32,
    set_layouts: *const VkDescriptorSetLayout,
}

#[repr(C)]
struct VkDescriptorImageInfo {
    sampler: VkSampler,
    image_view: VkImageView,
    image_layout: i32,
}

#[repr(C)]
struct VkSamplerCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    mag_filter: i32,
    min_filter: i32,
    mipmap_mode: i32,
    address_mode_u: i32,
    address_mode_v: i32,
    address_mode_w: i32,
    mip_lod_bias: f32,
    anisotropy_enable: u32,
    max_anisotropy: f32,
    compare_enable: u32,
    compare_op: i32,
    min_lod: f32,
    max_lod: f32,
    border_color: i32,
    unnormalized_coordinates: u32,
}

#[repr(C)]
struct VkDescriptorBufferInfo {
    buffer: VkBuffer,
    offset: u64,
    range: u64,
}

#[repr(C)]
struct VkWriteDescriptorSet {
    s_type: i32,
    p_next: *const c_void,
    dst_set: VkDescriptorSet,
    dst_binding: u32,
    dst_array_element: u32,
    descriptor_count: u32,
    descriptor_type: i32,
    image_info: *const VkDescriptorImageInfo,
    buffer_info: *const VkDescriptorBufferInfo,
    texel_buffer_view: *const VkBufferView,
}

#[repr(C)]
struct VkPushConstantRange {
    stage_flags: u32,
    offset: u32,
    size: u32,
}

#[repr(C)]
struct VkPipelineLayoutCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    set_layout_count: u32,
    set_layouts: *const VkDescriptorSetLayout,
    push_constant_range_count: u32,
    push_constant_ranges: *const VkPushConstantRange,
}

#[repr(C)]
struct VkShaderModuleCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    code_size: usize,
    code: *const u32,
}

#[repr(C)]
struct VkPipelineShaderStageCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    stage: u32,
    module: VkShaderModule,
    name: *const c_char,
    specialization_info: *const c_void,
}

#[repr(C)]
struct VkComputePipelineCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    stage: VkPipelineShaderStageCreateInfo,
    layout: VkPipelineLayout,
    base_pipeline_handle: VkPipeline,
    base_pipeline_index: i32,
}

type CreateImage = unsafe extern "system" fn(
    VkDevice,
    *const VkImageCreateInfo,
    *const VkAllocationCallbacks,
    *mut VkImage,
) -> VkResult;
type DestroyImage = unsafe extern "system" fn(VkDevice, VkImage, *const VkAllocationCallbacks);
type GetImageMemoryRequirements =
    unsafe extern "system" fn(VkDevice, VkImage, *mut VkMemoryRequirements);
type BindImageMemory =
    unsafe extern "system" fn(VkDevice, VkImage, VkDeviceMemory, u64) -> VkResult;
type CreateImageView = unsafe extern "system" fn(
    VkDevice,
    *const VkImageViewCreateInfo,
    *const VkAllocationCallbacks,
    *mut VkImageView,
) -> VkResult;
type DestroyImageView =
    unsafe extern "system" fn(VkDevice, VkImageView, *const VkAllocationCallbacks);
type CreateDescriptorSetLayout = unsafe extern "system" fn(
    VkDevice,
    *const VkDescriptorSetLayoutCreateInfo,
    *const VkAllocationCallbacks,
    *mut VkDescriptorSetLayout,
) -> VkResult;
type DestroyDescriptorSetLayout =
    unsafe extern "system" fn(VkDevice, VkDescriptorSetLayout, *const VkAllocationCallbacks);
type CreateDescriptorPool = unsafe extern "system" fn(
    VkDevice,
    *const VkDescriptorPoolCreateInfo,
    *const VkAllocationCallbacks,
    *mut VkDescriptorPool,
) -> VkResult;
type DestroyDescriptorPool =
    unsafe extern "system" fn(VkDevice, VkDescriptorPool, *const VkAllocationCallbacks);
type AllocateDescriptorSets = unsafe extern "system" fn(
    VkDevice,
    *const VkDescriptorSetAllocateInfo,
    *mut VkDescriptorSet,
) -> VkResult;
type UpdateDescriptorSets =
    unsafe extern "system" fn(VkDevice, u32, *const VkWriteDescriptorSet, u32, *const c_void);
type CreatePipelineLayout = unsafe extern "system" fn(
    VkDevice,
    *const VkPipelineLayoutCreateInfo,
    *const VkAllocationCallbacks,
    *mut VkPipelineLayout,
) -> VkResult;
type DestroyPipelineLayout =
    unsafe extern "system" fn(VkDevice, VkPipelineLayout, *const VkAllocationCallbacks);
type CreateShaderModule = unsafe extern "system" fn(
    VkDevice,
    *const VkShaderModuleCreateInfo,
    *const VkAllocationCallbacks,
    *mut VkShaderModule,
) -> VkResult;
type DestroyShaderModule =
    unsafe extern "system" fn(VkDevice, VkShaderModule, *const VkAllocationCallbacks);
type CreateComputePipelines = unsafe extern "system" fn(
    VkDevice,
    VkPipelineCache,
    u32,
    *const VkComputePipelineCreateInfo,
    *const VkAllocationCallbacks,
    *mut VkPipeline,
) -> VkResult;
type DestroyPipeline =
    unsafe extern "system" fn(VkDevice, VkPipeline, *const VkAllocationCallbacks);
type CreateSampler = unsafe extern "system" fn(
    VkDevice,
    *const VkSamplerCreateInfo,
    *const VkAllocationCallbacks,
    *mut VkSampler,
) -> VkResult;
type DestroySampler = unsafe extern "system" fn(VkDevice, VkSampler, *const VkAllocationCallbacks);

#[derive(Clone, Copy)]
struct Functions {
    create_image: CreateImage,
    destroy_image: DestroyImage,
    get_image_memory_requirements: GetImageMemoryRequirements,
    bind_image_memory: BindImageMemory,
    get_memory_properties: GetPhysicalDeviceMemoryProperties,
    allocate_memory: AllocateMemory,
    free_memory: FreeMemory,
    create_image_view: CreateImageView,
    destroy_image_view: DestroyImageView,
    create_descriptor_set_layout: CreateDescriptorSetLayout,
    destroy_descriptor_set_layout: DestroyDescriptorSetLayout,
    create_descriptor_pool: CreateDescriptorPool,
    destroy_descriptor_pool: DestroyDescriptorPool,
    allocate_descriptor_sets: AllocateDescriptorSets,
    update_descriptor_sets: UpdateDescriptorSets,
    create_pipeline_layout: CreatePipelineLayout,
    destroy_pipeline_layout: DestroyPipelineLayout,
    create_shader_module: CreateShaderModule,
    destroy_shader_module: DestroyShaderModule,
    create_compute_pipelines: CreateComputePipelines,
    destroy_pipeline: DestroyPipeline,
    create_sampler: CreateSampler,
    destroy_sampler: DestroySampler,
    get_fd_properties: GetMemoryFdProperties,
}

#[derive(Default)]
struct ImageResource {
    image: VkImage,
    memory: VkDeviceMemory,
    view: VkImageView,
    owns_image: bool,
    owns_memory: bool,
    owns_view: bool,
}

#[derive(Default)]
struct ConversionSlot {
    luma: ImageResource,
    chroma: ImageResource,
    encode_input: ImageResource,
    dpb: ImageResource,
    dpb_slot_index: u32,
    dpb_layer: u32,
    descriptor_set: VkDescriptorSet,
}

struct PendingFrame {
    timestamp_ns: u64,
    duration_ns: u64,
    requested_keyframe: bool,
    imported: VulkanVideoImportedBuffer,
}

pub struct VulkanVideoH264Encoder {
    request: VulkanVideoH264Request,
    // A timeout cannot destroy pending native children or unload their driver.
    // These owners are dropped explicitly only after completion is proven.
    parameters: ManuallyDrop<VulkanVideoH264Parameters>,
    functions: Functions,
    slots: Vec<ConversionSlot>,
    descriptor_set_layout: VkDescriptorSetLayout,
    descriptor_pool: VkDescriptorPool,
    pipeline_layout: VkPipelineLayout,
    pipeline: VkPipeline,
    sampler: VkSampler,
    input_mode: VulkanVideoInputMode,
    execution: ManuallyDrop<ExecutionResources>,
    encode: ManuallyDrop<EncodeResources>,
    pending: Vec<Option<PendingFrame>>,
    next_slot: usize,
    frame_index: u64,
    gop_position: u32,
    idr_pic_id: u16,
    previous_reference_slot: Option<usize>,
    previous_reference_was_idr: bool,
    initialized_slots: Vec<bool>,
    poisoned: bool,
}

impl VulkanVideoH264Parameters {
    /// Build the production encoder from Redunar's checked-in, release-verified
    /// compute shader. The application performs no runtime shader compilation.
    ///
    /// # Errors
    ///
    /// Returns a device error when the fixed conversion or encode resources
    /// cannot be created.
    pub fn create_production_encoder(
        self,
    ) -> Result<VulkanVideoH264Encoder, VulkanVideoDeviceError> {
        self.create_encoder_for_mode(REPLAY_CONVERSION_SPIRV, VulkanVideoInputMode::LinearBuffer)
    }

    /// Build the hardware encoder with direct DRM-image sampling for KMS
    /// framebuffer DMA-BUFs.
    ///
    /// # Errors
    ///
    /// Returns a device error if the image-input pipeline cannot be created.
    pub fn create_production_kms_encoder(
        self,
    ) -> Result<VulkanVideoH264Encoder, VulkanVideoDeviceError> {
        self.create_encoder_for_mode(REPLAY_KMS_CONVERSION_SPIRV, VulkanVideoInputMode::DrmImage)
    }

    /// Allocate the fixed GPU conversion images and build the compute pipeline.
    /// The SPIR-V must be generated from Redunar's adjacent checked-in GLSL.
    ///
    /// # Errors
    ///
    /// Returns [`VulkanVideoDeviceError`] for malformed SPIR-V or any image,
    /// descriptor, layout, shader, or pipeline creation failure. Partial
    /// resources are released transactionally.
    pub fn create_encoder(
        self,
        compute_spirv: &[u8],
    ) -> Result<VulkanVideoH264Encoder, VulkanVideoDeviceError> {
        self.create_encoder_for_mode(compute_spirv, VulkanVideoInputMode::LinearBuffer)
    }

    fn create_encoder_for_mode(
        self,
        compute_spirv: &[u8],
        input_mode: VulkanVideoInputMode,
    ) -> Result<VulkanVideoH264Encoder, VulkanVideoDeviceError> {
        let words = spirv_words(compute_spirv)?;
        // SAFETY: this consumes the unique session-parameter owner and every
        // native resource created by the helper is transactionally guarded.
        unsafe { create_encoder(self, &words, input_mode) }
    }
}

impl VulkanVideoH264Encoder {
    #[must_use]
    pub const fn request(&self) -> VulkanVideoH264Request {
        self.request
    }

    #[must_use]
    pub fn encoded_parameters(&self) -> &[u8] {
        self.parameters.encoded_parameters()
    }

    #[must_use]
    pub fn descriptor_sets(&self) -> impl ExactSizeIterator<Item = u64> + '_ {
        self.slots.iter().map(|slot| slot.descriptor_set)
    }

    /// Submit one DMA-BUF through conversion and hardware encoding. With two
    /// fixed slots, the call returns a previously completed access unit when
    /// available while the new frame remains in flight.
    ///
    /// # Errors
    ///
    /// Returns a device error when import, conversion, submission, or bounded
    /// output collection fails. Submission failures poison this encoder.
    pub fn encode_frame(
        &mut self,
        frame: VulkanVideoDmaBufFrame,
        force_keyframe: bool,
    ) -> Result<Vec<VulkanVideoEncodedAccessUnit>, VulkanVideoDeviceError> {
        execution::encode_frame(self, frame, force_keyframe)
    }

    /// Wait for and collect every submitted access unit in timestamp order.
    ///
    /// # Errors
    ///
    /// Returns a device error when a completion fence, feedback query, or
    /// encoded output cannot be validated.
    pub fn drain(&mut self) -> Result<Vec<VulkanVideoEncodedAccessUnit>, VulkanVideoDeviceError> {
        execution::drain(self)
    }

    /// Retire the owned encoder after a single bounded completion wait.
    ///
    /// # Errors
    ///
    /// Failure retains all native resources until process exit. The caller
    /// must preserve this failure and withhold producer-release acknowledgements.
    pub fn shutdown(mut self) -> Result<(), VulkanVideoDeviceError> {
        // SAFETY: this consumes the sole owner and no new submission can race.
        let result = unsafe { self.execution.shutdown() };
        if result.is_err() {
            ENCODER_TEARDOWN_INCOMPLETE.store(true, std::sync::atomic::Ordering::Release);
            mem::forget(self);
        }
        result
    }
}

#[repr(C)]
struct VkExternalMemoryImageCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    handle_types: u32,
}

#[repr(C)]
struct VkSubresourceLayout {
    offset: u64,
    size: u64,
    row_pitch: u64,
    array_pitch: u64,
    depth_pitch: u64,
}

#[repr(C)]
struct VkImageDrmFormatModifierExplicitCreateInfoExt {
    s_type: i32,
    p_next: *const c_void,
    drm_format_modifier: u64,
    drm_format_modifier_plane_count: u32,
    plane_layouts: *const VkSubresourceLayout,
}

/// Import one packed single-plane KMS framebuffer as a sampled Vulkan image.
///
/// # Safety
///
/// The caller keeps the owning Vulkan session live for the returned resource.
pub(super) unsafe fn import_drm_image(
    owner: &VulkanVideoH264Parameters,
    frame: VulkanVideoDmaBufFrame,
) -> Result<VulkanVideoImportedBuffer, VulkanVideoDeviceError> {
    const VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_IMAGE_CREATE_INFO: i32 = 1_000_072_001;
    const VK_STRUCTURE_TYPE_IMAGE_DRM_FORMAT_MODIFIER_EXPLICIT_CREATE_INFO_EXT: i32 = 1_000_158_004;
    let functions = unsafe { load_functions(owner) }?;
    let VulkanVideoDmaBufFrame {
        file,
        width,
        height,
        format,
        offset,
        stride,
        timestamp_ns,
        duration_ns,
        allocation_size,
        input_mode,
        drm_modifier,
    } = frame;
    if input_mode != VulkanVideoInputMode::DrmImage || allocation_size == 0 {
        return Err(VulkanVideoDeviceError::InvalidDmaBufFrame);
    }
    let vk_format = match format {
        VulkanPackedPixelFormat::Rgba8 => VK_FORMAT_R8G8B8A8_UNORM,
        VulkanPackedPixelFormat::Bgra8 => VK_FORMAT_B8G8R8A8_UNORM,
        VulkanPackedPixelFormat::A2b10g10r10 | VulkanPackedPixelFormat::A2r10g10b10 => {
            return Err(VulkanVideoDeviceError::InvalidDmaBufFrame);
        }
    };
    let device = owner.session.device.device_address as VkDevice;
    let physical_device = owner.session.device.physical_device_address as VkPhysicalDevice;
    let raw_fd = file.into_raw_fd();
    let mut fd_properties = VkMemoryFdPropertiesKhr {
        s_type: VK_STRUCTURE_TYPE_MEMORY_FD_PROPERTIES_KHR,
        p_next: ptr::null_mut(),
        memory_type_bits: 0,
    };
    let result = unsafe {
        (functions.get_fd_properties)(
            device,
            VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
            raw_fd,
            &raw mut fd_properties,
        )
    };
    if result != VK_SUCCESS {
        // SAFETY: the property query never consumes the descriptor.
        drop(unsafe { File::from_raw_fd(raw_fd) });
        return Err(VulkanVideoDeviceError::DmaBufPropertiesFailed(result));
    }
    let plane_layout = VkSubresourceLayout {
        offset: u64::from(offset),
        size: allocation_size.saturating_sub(u64::from(offset)),
        row_pitch: u64::from(stride),
        array_pitch: 0,
        depth_pitch: 0,
    };
    let external = VkExternalMemoryImageCreateInfo {
        s_type: VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_IMAGE_CREATE_INFO,
        p_next: ptr::null(),
        handle_types: VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
    };
    let modifier = VkImageDrmFormatModifierExplicitCreateInfoExt {
        s_type: VK_STRUCTURE_TYPE_IMAGE_DRM_FORMAT_MODIFIER_EXPLICIT_CREATE_INFO_EXT,
        p_next: (&raw const external).cast(),
        drm_format_modifier: drm_modifier,
        drm_format_modifier_plane_count: 1,
        plane_layouts: &raw const plane_layout,
    };
    let image_info = VkImageCreateInfo {
        s_type: VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
        p_next: (&raw const modifier).cast(),
        flags: 0,
        image_type: VK_IMAGE_TYPE_2D,
        format: vk_format,
        extent: VkExtent3d {
            width,
            height,
            depth: 1,
        },
        mip_levels: 1,
        array_layers: 1,
        samples: VK_SAMPLE_COUNT_1_BIT,
        tiling: VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT,
        usage: VK_IMAGE_USAGE_SAMPLED_BIT,
        sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        queue_family_indices: ptr::null(),
        initial_layout: VK_IMAGE_LAYOUT_UNDEFINED,
    };
    let mut image = 0;
    let result = unsafe {
        (functions.create_image)(device, &raw const image_info, ptr::null(), &raw mut image)
    };
    if result != VK_SUCCESS || image == 0 {
        // SAFETY: image creation failed before descriptor import.
        drop(unsafe { File::from_raw_fd(raw_fd) });
        return Err(VulkanVideoDeviceError::ImageCreationFailed(result));
    }
    let mut requirements = VkMemoryRequirements {
        size: 0,
        alignment: 0,
        memory_type_bits: 0,
    };
    unsafe { (functions.get_image_memory_requirements)(device, image, &raw mut requirements) };
    if requirements.size == 0 || requirements.size > allocation_size {
        unsafe { (functions.destroy_image)(device, image, ptr::null()) };
        drop(unsafe { File::from_raw_fd(raw_fd) });
        return Err(VulkanVideoDeviceError::InvalidDmaBufFrame);
    }
    let mut memory_properties = VkPhysicalDeviceMemoryProperties::default();
    unsafe { (functions.get_memory_properties)(physical_device, &raw mut memory_properties) };
    let compatible = requirements.memory_type_bits & fd_properties.memory_type_bits;
    let memory_type_index =
        compatible_memory_type(&memory_properties, compatible).ok_or_else(|| {
            unsafe { (functions.destroy_image)(device, image, ptr::null()) };
            drop(unsafe { File::from_raw_fd(raw_fd) });
            VulkanVideoDeviceError::ImportedMemoryTypeUnavailable
        })?;
    let import = VkImportMemoryFdInfoKhr {
        s_type: VK_STRUCTURE_TYPE_IMPORT_MEMORY_FD_INFO_KHR,
        p_next: ptr::null(),
        handle_type: VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
        fd: raw_fd,
    };
    let dedicated = VkMemoryDedicatedAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_MEMORY_DEDICATED_ALLOCATE_INFO,
        p_next: (&raw const import).cast(),
        image,
        buffer: 0,
    };
    let allocation_info = VkMemoryAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: (&raw const dedicated).cast(),
        allocation_size: requirements.size,
        memory_type_index,
    };
    let mut memory = 0;
    let result = unsafe {
        (functions.allocate_memory)(
            device,
            &raw const allocation_info,
            ptr::null(),
            &raw mut memory,
        )
    };
    if result != VK_SUCCESS || memory == 0 {
        unsafe { (functions.destroy_image)(device, image, ptr::null()) };
        drop(unsafe { File::from_raw_fd(raw_fd) });
        return Err(VulkanVideoDeviceError::ImportedMemoryAllocationFailed(
            result,
        ));
    }
    let result = unsafe { (functions.bind_image_memory)(device, image, memory, 0) };
    if result != VK_SUCCESS {
        unsafe {
            (functions.destroy_image)(device, image, ptr::null());
            (functions.free_memory)(device, memory, ptr::null());
        }
        return Err(VulkanVideoDeviceError::ImageMemoryBindFailed(result));
    }
    let view_info = VkImageViewCreateInfo {
        s_type: VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        image,
        view_type: VK_IMAGE_VIEW_TYPE_2D,
        format: vk_format,
        components: VkComponentMapping {
            r: VK_COMPONENT_SWIZZLE_IDENTITY,
            g: VK_COMPONENT_SWIZZLE_IDENTITY,
            b: VK_COMPONENT_SWIZZLE_IDENTITY,
            a: VK_COMPONENT_SWIZZLE_IDENTITY,
        },
        subresource_range: VkImageSubresourceRange {
            aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        },
    };
    let mut view = 0;
    let result = unsafe {
        (functions.create_image_view)(device, &raw const view_info, ptr::null(), &raw mut view)
    };
    if result != VK_SUCCESS || view == 0 {
        unsafe {
            (functions.destroy_image)(device, image, ptr::null());
            (functions.free_memory)(device, memory, ptr::null());
        }
        return Err(VulkanVideoDeviceError::ImageViewCreationFailed(result));
    }
    Ok(VulkanVideoImportedBuffer {
        device_address: owner.session.device.device_address,
        resource: VulkanVideoImportedResource::DrmImage {
            image,
            view,
            memory,
            destroy_image: functions.destroy_image,
            destroy_image_view: functions.destroy_image_view,
            free_memory: functions.free_memory,
        },
        width,
        height,
        format,
        offset_words: 0,
        stride_words: 0,
        timestamp_ns,
        duration_ns,
    })
}

impl Drop for VulkanVideoH264Encoder {
    fn drop(&mut self) {
        let device = self.parameters.session.device.device_address as VkDevice;
        // SAFETY: all resources belong to this live device and are destroyed
        // in dependency order before nested session parameters are dropped.
        unsafe {
            if self.execution.shutdown().is_err() {
                // Bound retained resources to the failed encoder generation.
                // No later open is allowed in this process. Imported allocations,
                // session/device/instance and dlopen lease survive until exit.
                ENCODER_TEARDOWN_INCOMPLETE.store(true, std::sync::atomic::Ordering::Release);
                for pending in self.pending.drain(..).flatten() {
                    mem::forget(pending);
                }
                return;
            }
            self.pending.clear();
            self.encode.destroy();
            if self.pipeline != 0 {
                (self.functions.destroy_pipeline)(device, self.pipeline, ptr::null());
                self.pipeline = 0;
            }
            if self.pipeline_layout != 0 {
                (self.functions.destroy_pipeline_layout)(device, self.pipeline_layout, ptr::null());
                self.pipeline_layout = 0;
            }
            if self.descriptor_pool != 0 {
                (self.functions.destroy_descriptor_pool)(device, self.descriptor_pool, ptr::null());
                self.descriptor_pool = 0;
                for slot in &mut self.slots {
                    slot.descriptor_set = 0;
                }
            }
            if self.sampler != 0 {
                (self.functions.destroy_sampler)(device, self.sampler, ptr::null());
                self.sampler = 0;
            }
            if self.descriptor_set_layout != 0 {
                (self.functions.destroy_descriptor_set_layout)(
                    device,
                    self.descriptor_set_layout,
                    ptr::null(),
                );
                self.descriptor_set_layout = 0;
            }
            for slot in self.slots.iter_mut().rev() {
                destroy_image(device, &mut slot.dpb, self.functions);
                destroy_image(device, &mut slot.encode_input, self.functions);
                destroy_image(device, &mut slot.chroma, self.functions);
                destroy_image(device, &mut slot.luma, self.functions);
            }
            ManuallyDrop::drop(&mut self.encode);
            ManuallyDrop::drop(&mut self.execution);
            ManuallyDrop::drop(&mut self.parameters);
        }
    }
}

fn spirv_words(bytes: &[u8]) -> Result<Vec<u32>, VulkanVideoDeviceError> {
    let first_word = bytes
        .get(..4)
        .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
        .map(u32::from_le_bytes);
    if bytes.is_empty()
        || bytes.len() > MAX_COMPUTE_SHADER_BYTES
        || !bytes.len().is_multiple_of(4)
        || first_word != Some(SPIRV_MAGIC)
    {
        return Err(VulkanVideoDeviceError::InvalidComputeShader);
    }
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| u32::from_le_bytes(*chunk))
        .collect())
}

unsafe fn load_functions(
    owner: &VulkanVideoH264Parameters,
) -> Result<Functions, VulkanVideoDeviceError> {
    macro_rules! load {
        ($symbol:literal, $ty:ty) => {{
            let name = concat!($symbol, "\0");
            let Some(raw) =
                (unsafe { resolve_device_command(&owner.session.device, name.as_ptr().cast()) })
            else {
                return Err(VulkanVideoDeviceError::DeviceSymbolUnavailable($symbol));
            };
            // SAFETY: the command name fixes the exact function ABI.
            unsafe { mem::transmute::<unsafe extern "system" fn(), $ty>(raw) }
        }};
    }
    Ok(Functions {
        create_image: load!("vkCreateImage", CreateImage),
        destroy_image: load!("vkDestroyImage", DestroyImage),
        get_image_memory_requirements: load!(
            "vkGetImageMemoryRequirements",
            GetImageMemoryRequirements
        ),
        bind_image_memory: load!("vkBindImageMemory", BindImageMemory),
        get_memory_properties: load!(
            "vkGetPhysicalDeviceMemoryProperties",
            GetPhysicalDeviceMemoryProperties
        ),
        allocate_memory: load!("vkAllocateMemory", AllocateMemory),
        free_memory: load!("vkFreeMemory", FreeMemory),
        create_image_view: load!("vkCreateImageView", CreateImageView),
        destroy_image_view: load!("vkDestroyImageView", DestroyImageView),
        create_descriptor_set_layout: load!(
            "vkCreateDescriptorSetLayout",
            CreateDescriptorSetLayout
        ),
        destroy_descriptor_set_layout: load!(
            "vkDestroyDescriptorSetLayout",
            DestroyDescriptorSetLayout
        ),
        create_descriptor_pool: load!("vkCreateDescriptorPool", CreateDescriptorPool),
        destroy_descriptor_pool: load!("vkDestroyDescriptorPool", DestroyDescriptorPool),
        allocate_descriptor_sets: load!("vkAllocateDescriptorSets", AllocateDescriptorSets),
        update_descriptor_sets: load!("vkUpdateDescriptorSets", UpdateDescriptorSets),
        create_pipeline_layout: load!("vkCreatePipelineLayout", CreatePipelineLayout),
        destroy_pipeline_layout: load!("vkDestroyPipelineLayout", DestroyPipelineLayout),
        create_shader_module: load!("vkCreateShaderModule", CreateShaderModule),
        destroy_shader_module: load!("vkDestroyShaderModule", DestroyShaderModule),
        create_compute_pipelines: load!("vkCreateComputePipelines", CreateComputePipelines),
        destroy_pipeline: load!("vkDestroyPipeline", DestroyPipeline),
        create_sampler: load!("vkCreateSampler", CreateSampler),
        destroy_sampler: load!("vkDestroySampler", DestroySampler),
        get_fd_properties: load!("vkGetMemoryFdPropertiesKHR", GetMemoryFdProperties),
    })
}

unsafe fn create_encoder(
    parameters: VulkanVideoH264Parameters,
    compute_words: &[u32],
    input_mode: VulkanVideoInputMode,
) -> Result<VulkanVideoH264Encoder, VulkanVideoDeviceError> {
    let functions = unsafe { load_functions(&parameters) }?;
    let device = parameters.session.device.device_address as VkDevice;
    let physical_device = parameters.session.device.physical_device_address as VkPhysicalDevice;
    let request = parameters.request();
    let queue_families = [
        parameters
            .session
            .device
            .candidate
            .compute_queue_family_index,
        parameters.session.device.candidate.queue_family_index,
    ];
    let mut build = BuildGuard::new(device, functions);
    let encode_families: &[u32] = if queue_families[0] == queue_families[1] {
        &queue_families[..1]
    } else {
        &queue_families
    };
    for _ in 0..CONVERSION_SLOT_COUNT {
        let luma = unsafe {
            create_image(
                device,
                physical_device,
                functions,
                VK_FORMAT_R8_UNORM,
                request.width,
                request.height,
                VK_IMAGE_USAGE_STORAGE_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
                &queue_families[..1],
                1,
                0,
            )
        }?;
        build.slots.push(ConversionSlot {
            luma,
            ..ConversionSlot::default()
        });
        let slot = build
            .slots
            .last_mut()
            .expect("the conversion slot was just inserted");
        slot.chroma = unsafe {
            create_image(
                device,
                physical_device,
                functions,
                VK_FORMAT_R8G8_UNORM,
                request.width / 2,
                request.height / 2,
                VK_IMAGE_USAGE_STORAGE_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
                &queue_families[..1],
                1,
                0,
            )
        }?;
        slot.encode_input = unsafe {
            create_image(
                device,
                physical_device,
                functions,
                parameters.session.device.candidate.encode_input_format,
                request.width,
                request.height,
                VK_IMAGE_USAGE_TRANSFER_DST_BIT | VK_IMAGE_USAGE_VIDEO_ENCODE_SRC_BIT_KHR,
                encode_families,
                1,
                0,
            )
        }?;
    }
    if parameters
        .session
        .device
        .candidate
        .separate_reference_images
    {
        for (index, slot) in build.slots.iter_mut().take(DPB_SLOT_COUNT).enumerate() {
            slot.dpb = unsafe {
                create_image(
                    device,
                    physical_device,
                    functions,
                    parameters.session.device.candidate.dpb_format,
                    request.width,
                    request.height,
                    VK_IMAGE_USAGE_VIDEO_ENCODE_DPB_BIT_KHR,
                    &queue_families[1..],
                    1,
                    0,
                )
            }?;
            slot.dpb_slot_index = u32::try_from(index).unwrap_or(0);
            slot.dpb_layer = 0;
        }
        let shared_dpb = [
            (build.slots[0].dpb.image, build.slots[0].dpb.view),
            (build.slots[1].dpb.image, build.slots[1].dpb.view),
        ];
        for (index, slot) in build.slots.iter_mut().enumerate().skip(DPB_SLOT_COUNT) {
            let dpb_index = index % DPB_SLOT_COUNT;
            slot.dpb = ImageResource {
                image: shared_dpb[dpb_index].0,
                memory: 0,
                view: shared_dpb[dpb_index].1,
                owns_image: false,
                owns_memory: false,
                owns_view: false,
            };
            slot.dpb_slot_index = u32::try_from(dpb_index).unwrap_or(0);
            slot.dpb_layer = 0;
        }
    } else {
        let layered = unsafe {
            create_image(
                device,
                physical_device,
                functions,
                parameters.session.device.candidate.dpb_format,
                request.width,
                request.height,
                VK_IMAGE_USAGE_VIDEO_ENCODE_DPB_BIT_KHR,
                &queue_families[1..],
                u32::try_from(DPB_SLOT_COUNT).unwrap_or(2),
                0,
            )
        }?;
        let image = layered.image;
        let view = layered.view;
        build.slots[0].dpb = layered;
        for (index, slot) in build.slots.iter_mut().enumerate() {
            let dpb_index = index % DPB_SLOT_COUNT;
            if index != 0 {
                slot.dpb = ImageResource {
                    image,
                    memory: 0,
                    view,
                    owns_image: false,
                    owns_memory: false,
                    owns_view: false,
                };
            }
            slot.dpb_slot_index = u32::try_from(dpb_index).unwrap_or(0);
            slot.dpb_layer = u32::try_from(dpb_index).unwrap_or(0);
        }
    }

    let source_descriptor = match input_mode {
        VulkanVideoInputMode::LinearBuffer => VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
        VulkanVideoInputMode::DrmImage => VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,
    };
    let bindings = [
        descriptor_binding(0, source_descriptor),
        descriptor_binding(1, VK_DESCRIPTOR_TYPE_STORAGE_IMAGE),
        descriptor_binding(2, VK_DESCRIPTOR_TYPE_STORAGE_IMAGE),
    ];
    let descriptor_layout_info = VkDescriptorSetLayoutCreateInfo {
        s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        binding_count: u32::try_from(bindings.len()).unwrap_or(0),
        bindings: bindings.as_ptr(),
    };
    let result = unsafe {
        (functions.create_descriptor_set_layout)(
            device,
            &raw const descriptor_layout_info,
            ptr::null(),
            &raw mut build.descriptor_set_layout,
        )
    };
    if result != VK_SUCCESS || build.descriptor_set_layout == 0 {
        return Err(VulkanVideoDeviceError::DescriptorSetLayoutCreationFailed(
            result,
        ));
    }
    let pool_sizes = [
        VkDescriptorPoolSize {
            descriptor_type: source_descriptor,
            descriptor_count: u32::try_from(CONVERSION_SLOT_COUNT).unwrap_or(0),
        },
        VkDescriptorPoolSize {
            descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_IMAGE,
            descriptor_count: u32::try_from(CONVERSION_SLOT_COUNT.saturating_mul(2)).unwrap_or(0),
        },
    ];
    if input_mode == VulkanVideoInputMode::DrmImage {
        let sampler_info = VkSamplerCreateInfo {
            s_type: VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            mag_filter: 0,
            min_filter: 0,
            mipmap_mode: 0,
            address_mode_u: 2,
            address_mode_v: 2,
            address_mode_w: 2,
            mip_lod_bias: 0.0,
            anisotropy_enable: 0,
            max_anisotropy: 1.0,
            compare_enable: 0,
            compare_op: 7,
            min_lod: 0.0,
            max_lod: 0.0,
            border_color: 0,
            unnormalized_coordinates: 0,
        };
        let result = unsafe {
            (functions.create_sampler)(
                device,
                &raw const sampler_info,
                ptr::null(),
                &raw mut build.sampler,
            )
        };
        if result != VK_SUCCESS || build.sampler == 0 {
            return Err(VulkanVideoDeviceError::SamplerCreationFailed(result));
        }
    }
    let pool_info = VkDescriptorPoolCreateInfo {
        s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        max_sets: u32::try_from(CONVERSION_SLOT_COUNT).unwrap_or(0),
        pool_size_count: u32::try_from(pool_sizes.len()).unwrap_or(0),
        pool_sizes: pool_sizes.as_ptr(),
    };
    let result = unsafe {
        (functions.create_descriptor_pool)(
            device,
            &raw const pool_info,
            ptr::null(),
            &raw mut build.descriptor_pool,
        )
    };
    if result != VK_SUCCESS || build.descriptor_pool == 0 {
        return Err(VulkanVideoDeviceError::DescriptorPoolCreationFailed(result));
    }
    let set_layouts = [build.descriptor_set_layout; CONVERSION_SLOT_COUNT];
    let allocate_info = VkDescriptorSetAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
        p_next: ptr::null(),
        descriptor_pool: build.descriptor_pool,
        descriptor_set_count: u32::try_from(CONVERSION_SLOT_COUNT).unwrap_or(0),
        set_layouts: set_layouts.as_ptr(),
    };
    let mut descriptor_sets = [0; CONVERSION_SLOT_COUNT];
    let result = unsafe {
        (functions.allocate_descriptor_sets)(
            device,
            &raw const allocate_info,
            descriptor_sets.as_mut_ptr(),
        )
    };
    if result != VK_SUCCESS || descriptor_sets.contains(&0) {
        return Err(VulkanVideoDeviceError::DescriptorSetAllocationFailed(
            result,
        ));
    }
    let mut output_images = Vec::with_capacity(CONVERSION_SLOT_COUNT * 2);
    for slot in &build.slots {
        output_images.push(VkDescriptorImageInfo {
            sampler: 0,
            image_view: slot.luma.view,
            image_layout: VK_IMAGE_LAYOUT_GENERAL,
        });
        output_images.push(VkDescriptorImageInfo {
            sampler: 0,
            image_view: slot.chroma.view,
            image_layout: VK_IMAGE_LAYOUT_GENERAL,
        });
    }
    let mut writes = Vec::with_capacity(CONVERSION_SLOT_COUNT * 2);
    for (slot_index, descriptor_set) in descriptor_sets.into_iter().enumerate() {
        build.slots[slot_index].descriptor_set = descriptor_set;
        writes.push(image_write(
            descriptor_set,
            1,
            &raw const output_images[slot_index * 2],
        ));
        writes.push(image_write(
            descriptor_set,
            2,
            &raw const output_images[slot_index * 2 + 1],
        ));
    }
    unsafe {
        (functions.update_descriptor_sets)(
            device,
            u32::try_from(writes.len()).unwrap_or(0),
            writes.as_ptr(),
            0,
            ptr::null(),
        );
    }

    let push_constant = VkPushConstantRange {
        stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
        offset: 0,
        size: PUSH_CONSTANT_BYTES,
    };
    let pipeline_layout_info = VkPipelineLayoutCreateInfo {
        s_type: VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        set_layout_count: 1,
        set_layouts: &raw const build.descriptor_set_layout,
        push_constant_range_count: 1,
        push_constant_ranges: &raw const push_constant,
    };
    let result = unsafe {
        (functions.create_pipeline_layout)(
            device,
            &raw const pipeline_layout_info,
            ptr::null(),
            &raw mut build.pipeline_layout,
        )
    };
    if result != VK_SUCCESS || build.pipeline_layout == 0 {
        return Err(VulkanVideoDeviceError::PipelineLayoutCreationFailed(result));
    }
    let shader_info = VkShaderModuleCreateInfo {
        s_type: VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        code_size: compute_words.len().saturating_mul(4),
        code: compute_words.as_ptr(),
    };
    let result = unsafe {
        (functions.create_shader_module)(
            device,
            &raw const shader_info,
            ptr::null(),
            &raw mut build.shader_module,
        )
    };
    if result != VK_SUCCESS || build.shader_module == 0 {
        return Err(VulkanVideoDeviceError::ShaderModuleCreationFailed(result));
    }
    let entry = b"main\0";
    let compute_info = VkComputePipelineCreateInfo {
        s_type: VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        stage: VkPipelineShaderStageCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            stage: VK_SHADER_STAGE_COMPUTE_BIT,
            module: build.shader_module,
            name: entry.as_ptr().cast(),
            specialization_info: ptr::null(),
        },
        layout: build.pipeline_layout,
        base_pipeline_handle: 0,
        base_pipeline_index: -1,
    };
    let result = unsafe {
        (functions.create_compute_pipelines)(
            device,
            0,
            1,
            &raw const compute_info,
            ptr::null(),
            &raw mut build.pipeline,
        )
    };
    if result != VK_SUCCESS || build.pipeline == 0 {
        return Err(VulkanVideoDeviceError::ComputePipelineCreationFailed(
            result,
        ));
    }
    unsafe { (functions.destroy_shader_module)(device, build.shader_module, ptr::null()) };
    build.shader_module = 0;
    let execution = unsafe { ExecutionResources::new(&parameters, CONVERSION_SLOT_COUNT) }?;
    let encode = unsafe { EncodeResources::new(&parameters, CONVERSION_SLOT_COUNT) }?;
    let resources = build.disarm();
    drop(build);
    Ok(VulkanVideoH264Encoder {
        request: parameters.request(),
        parameters: ManuallyDrop::new(parameters),
        functions,
        slots: resources.slots,
        descriptor_set_layout: resources.descriptor_set_layout,
        descriptor_pool: resources.descriptor_pool,
        pipeline_layout: resources.pipeline_layout,
        pipeline: resources.pipeline,
        sampler: resources.sampler,
        input_mode,
        execution: ManuallyDrop::new(execution),
        encode: ManuallyDrop::new(encode),
        pending: (0..CONVERSION_SLOT_COUNT).map(|_| None).collect(),
        next_slot: 0,
        frame_index: 0,
        gop_position: 0,
        idr_pic_id: 0,
        previous_reference_slot: None,
        previous_reference_was_idr: false,
        initialized_slots: vec![false; CONVERSION_SLOT_COUNT],
        poisoned: false,
    })
}

fn descriptor_binding(binding: u32, descriptor_type: i32) -> VkDescriptorSetLayoutBinding {
    VkDescriptorSetLayoutBinding {
        binding,
        descriptor_type,
        descriptor_count: 1,
        stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
        immutable_samplers: ptr::null(),
    }
}

fn image_write(
    set: VkDescriptorSet,
    binding: u32,
    image_info: *const VkDescriptorImageInfo,
) -> VkWriteDescriptorSet {
    VkWriteDescriptorSet {
        s_type: VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
        p_next: ptr::null(),
        dst_set: set,
        dst_binding: binding,
        dst_array_element: 0,
        descriptor_count: 1,
        descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_IMAGE,
        image_info,
        buffer_info: ptr::null(),
        texel_buffer_view: ptr::null(),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the internal Vulkan image constructor keeps native creation fields explicit"
)]
unsafe fn create_image(
    device: VkDevice,
    physical_device: VkPhysicalDevice,
    functions: Functions,
    format: i32,
    width: u32,
    height: u32,
    usage: u32,
    queue_families: &[u32],
    array_layers: u32,
    view_layer: u32,
) -> Result<ImageResource, VulkanVideoDeviceError> {
    let sharing_mode = if queue_families.len() > 1 {
        VK_SHARING_MODE_CONCURRENT
    } else {
        VK_SHARING_MODE_EXCLUSIVE
    };
    let h264_profile = VkVideoEncodeH264ProfileInfoKhr {
        s_type: VK_STRUCTURE_TYPE_VIDEO_ENCODE_H264_PROFILE_INFO_KHR,
        p_next: ptr::null(),
        std_profile_idc: STD_VIDEO_H264_PROFILE_IDC_HIGH,
    };
    let usage_profile = VkVideoEncodeUsageInfoKhr {
        s_type: VK_STRUCTURE_TYPE_VIDEO_ENCODE_USAGE_INFO_KHR,
        p_next: (&raw const h264_profile).cast(),
        video_usage_hints: VK_VIDEO_ENCODE_USAGE_RECORDING_BIT_KHR,
        video_content_hints: VK_VIDEO_ENCODE_CONTENT_RENDERED_BIT_KHR,
        tuning_mode: VK_VIDEO_ENCODE_TUNING_MODE_DEFAULT_KHR,
    };
    let video_profile = VkVideoProfileInfoKhr {
        s_type: VK_STRUCTURE_TYPE_VIDEO_PROFILE_INFO_KHR,
        p_next: (&raw const usage_profile).cast(),
        video_codec_operation: VK_VIDEO_CODEC_OPERATION_ENCODE_H264_BIT_KHR,
        chroma_subsampling: VK_VIDEO_CHROMA_SUBSAMPLING_420_BIT_KHR,
        luma_bit_depth: VK_VIDEO_COMPONENT_BIT_DEPTH_8_BIT_KHR,
        chroma_bit_depth: VK_VIDEO_COMPONENT_BIT_DEPTH_8_BIT_KHR,
    };
    let profile_list = VkVideoProfileListInfoKhr {
        s_type: VK_STRUCTURE_TYPE_VIDEO_PROFILE_LIST_INFO_KHR,
        p_next: ptr::null(),
        profile_count: 1,
        profiles: &raw const video_profile,
    };
    let is_video_image = usage
        & (VK_IMAGE_USAGE_VIDEO_ENCODE_SRC_BIT_KHR | VK_IMAGE_USAGE_VIDEO_ENCODE_DPB_BIT_KHR)
        != 0;
    let create_info = VkImageCreateInfo {
        s_type: VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
        p_next: if is_video_image {
            (&raw const profile_list).cast()
        } else {
            ptr::null()
        },
        flags: 0,
        image_type: VK_IMAGE_TYPE_2D,
        format,
        extent: VkExtent3d {
            width,
            height,
            depth: 1,
        },
        mip_levels: 1,
        array_layers,
        samples: VK_SAMPLE_COUNT_1_BIT,
        tiling: VK_IMAGE_TILING_OPTIMAL,
        usage,
        sharing_mode,
        queue_family_index_count: if sharing_mode == VK_SHARING_MODE_CONCURRENT {
            u32::try_from(queue_families.len()).unwrap_or(0)
        } else {
            0
        },
        queue_family_indices: if sharing_mode == VK_SHARING_MODE_CONCURRENT {
            queue_families.as_ptr()
        } else {
            ptr::null()
        },
        initial_layout: VK_IMAGE_LAYOUT_UNDEFINED,
    };
    let mut resource = ImageResource::default();
    let result = unsafe {
        (functions.create_image)(
            device,
            &raw const create_info,
            ptr::null(),
            &raw mut resource.image,
        )
    };
    if result != VK_SUCCESS || resource.image == 0 {
        return Err(VulkanVideoDeviceError::ImageCreationFailed(result));
    }
    resource.owns_image = true;
    let mut requirements = VkMemoryRequirements {
        size: 0,
        alignment: 0,
        memory_type_bits: 0,
    };
    unsafe {
        (functions.get_image_memory_requirements)(device, resource.image, &raw mut requirements);
    }
    let mut properties = VkPhysicalDeviceMemoryProperties::default();
    unsafe { (functions.get_memory_properties)(physical_device, &raw mut properties) };
    let memory_type_index = device_local_memory_type(&properties, requirements.memory_type_bits)
        .ok_or_else(|| {
            unsafe { destroy_image(device, &mut resource, functions) };
            VulkanVideoDeviceError::ImageMemoryTypeUnavailable
        })?;
    let allocation_info = VkMemoryAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: ptr::null(),
        allocation_size: requirements.size,
        memory_type_index,
    };
    let result = unsafe {
        (functions.allocate_memory)(
            device,
            &raw const allocation_info,
            ptr::null(),
            &raw mut resource.memory,
        )
    };
    if result != VK_SUCCESS || resource.memory == 0 {
        unsafe { destroy_image(device, &mut resource, functions) };
        return Err(VulkanVideoDeviceError::ImageMemoryAllocationFailed(result));
    }
    resource.owns_memory = true;
    let result =
        unsafe { (functions.bind_image_memory)(device, resource.image, resource.memory, 0) };
    if result != VK_SUCCESS {
        unsafe { destroy_image(device, &mut resource, functions) };
        return Err(VulkanVideoDeviceError::ImageMemoryBindFailed(result));
    }
    resource.view = unsafe {
        create_image_view(
            device,
            functions,
            resource.image,
            format,
            view_layer,
            array_layers,
        )
    }
    .inspect_err(|_| unsafe { destroy_image(device, &mut resource, functions) })?;
    resource.owns_view = true;
    Ok(resource)
}

unsafe fn create_image_view(
    device: VkDevice,
    functions: Functions,
    image: VkImage,
    format: i32,
    base_array_layer: u32,
    layer_count: u32,
) -> Result<VkImageView, VulkanVideoDeviceError> {
    let view_info = VkImageViewCreateInfo {
        s_type: VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        image,
        view_type: if layer_count > 1 {
            VK_IMAGE_VIEW_TYPE_2D_ARRAY
        } else {
            VK_IMAGE_VIEW_TYPE_2D
        },
        format,
        components: VkComponentMapping {
            r: VK_COMPONENT_SWIZZLE_IDENTITY,
            g: VK_COMPONENT_SWIZZLE_IDENTITY,
            b: VK_COMPONENT_SWIZZLE_IDENTITY,
            a: VK_COMPONENT_SWIZZLE_IDENTITY,
        },
        subresource_range: VkImageSubresourceRange {
            aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer,
            layer_count,
        },
    };
    let mut view = 0;
    let result = unsafe {
        (functions.create_image_view)(device, &raw const view_info, ptr::null(), &raw mut view)
    };
    if result != VK_SUCCESS || view == 0 {
        return Err(VulkanVideoDeviceError::ImageViewCreationFailed(result));
    }
    Ok(view)
}

unsafe fn destroy_image(device: VkDevice, resource: &mut ImageResource, functions: Functions) {
    if resource.view != 0 && resource.owns_view {
        unsafe { (functions.destroy_image_view)(device, resource.view, ptr::null()) };
        resource.view = 0;
    }
    if resource.image != 0 && resource.owns_image {
        unsafe { (functions.destroy_image)(device, resource.image, ptr::null()) };
        resource.image = 0;
    }
    if resource.memory != 0 && resource.owns_memory {
        unsafe { (functions.free_memory)(device, resource.memory, ptr::null()) };
        resource.memory = 0;
    }
}

struct BuiltResources {
    slots: Vec<ConversionSlot>,
    descriptor_set_layout: VkDescriptorSetLayout,
    descriptor_pool: VkDescriptorPool,
    pipeline_layout: VkPipelineLayout,
    pipeline: VkPipeline,
    sampler: VkSampler,
}

struct BuildGuard {
    device: VkDevice,
    functions: Functions,
    slots: Vec<ConversionSlot>,
    descriptor_set_layout: VkDescriptorSetLayout,
    descriptor_pool: VkDescriptorPool,
    pipeline_layout: VkPipelineLayout,
    shader_module: VkShaderModule,
    pipeline: VkPipeline,
    sampler: VkSampler,
    active: bool,
}

impl BuildGuard {
    fn new(device: VkDevice, functions: Functions) -> Self {
        Self {
            device,
            functions,
            slots: Vec::with_capacity(CONVERSION_SLOT_COUNT),
            descriptor_set_layout: 0,
            descriptor_pool: 0,
            pipeline_layout: 0,
            shader_module: 0,
            pipeline: 0,
            sampler: 0,
            active: true,
        }
    }

    fn disarm(&mut self) -> BuiltResources {
        self.active = false;
        BuiltResources {
            slots: mem::take(&mut self.slots),
            descriptor_set_layout: mem::take(&mut self.descriptor_set_layout),
            descriptor_pool: mem::take(&mut self.descriptor_pool),
            pipeline_layout: mem::take(&mut self.pipeline_layout),
            pipeline: mem::take(&mut self.pipeline),
            sampler: mem::take(&mut self.sampler),
        }
    }
}

impl Drop for BuildGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        // SAFETY: the guard owns every non-zero partial resource.
        unsafe {
            if self.pipeline != 0 {
                (self.functions.destroy_pipeline)(self.device, self.pipeline, ptr::null());
            }
            if self.shader_module != 0 {
                (self.functions.destroy_shader_module)(
                    self.device,
                    self.shader_module,
                    ptr::null(),
                );
            }
            if self.pipeline_layout != 0 {
                (self.functions.destroy_pipeline_layout)(
                    self.device,
                    self.pipeline_layout,
                    ptr::null(),
                );
            }
            if self.sampler != 0 {
                (self.functions.destroy_sampler)(self.device, self.sampler, ptr::null());
            }
            if self.descriptor_pool != 0 {
                (self.functions.destroy_descriptor_pool)(
                    self.device,
                    self.descriptor_pool,
                    ptr::null(),
                );
            }
            if self.descriptor_set_layout != 0 {
                (self.functions.destroy_descriptor_set_layout)(
                    self.device,
                    self.descriptor_set_layout,
                    ptr::null(),
                );
            }
            for slot in self.slots.iter_mut().rev() {
                destroy_image(self.device, &mut slot.dpb, self.functions);
                destroy_image(self.device, &mut slot.encode_input, self.functions);
                destroy_image(self.device, &mut slot.chroma, self.functions);
                destroy_image(self.device, &mut slot.luma, self.functions);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_quality_and_rate_control_are_configured_only_for_the_initial_frame() {
        assert_eq!(execution::initial_rate_control_flags(0), Some(0x7));
        assert_eq!(execution::initial_rate_control_flags(1), None);
        assert_eq!(execution::initial_rate_control_flags(u64::MAX), None);
    }

    #[test]
    fn compute_shader_input_is_strictly_bounded_and_word_aligned() {
        assert_eq!(
            spirv_words(&[]),
            Err(VulkanVideoDeviceError::InvalidComputeShader)
        );
        assert_eq!(
            spirv_words(&SPIRV_MAGIC.to_le_bytes()).expect("one valid word"),
            vec![SPIRV_MAGIC]
        );
    }

    #[test]
    fn kms_import_structures_match_the_public_vulkan_abi() {
        assert_eq!(mem::size_of::<VkSamplerCreateInfo>(), 80);
        assert_eq!(mem::size_of::<VkExternalMemoryImageCreateInfo>(), 24);
        assert_eq!(mem::size_of::<VkSubresourceLayout>(), 40);
        assert_eq!(
            mem::size_of::<VkImageDrmFormatModifierExplicitCreateInfoExt>(),
            40
        );
        assert_eq!(mem::size_of::<VkImageCreateInfo>(), 88);
        assert_eq!(mem::size_of::<VkImageViewCreateInfo>(), 80);
    }
}
