//! Vulkan rendering context and swapchain management.

use std::ffi::CStr;

use anyhow::{Context as _, Result, bail};
use ash::vk;
use fluidsim::SharedGpuContext;
use gpu_allocator::vulkan::Allocator;
use parking_lot::Mutex;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle};
use std::sync::Arc;
use winit::window::Window;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentModePreference {
    Auto,
    Fifo,
    Mailbox,
}

impl PresentModePreference {
    fn select(
        self,
        display_handle: RawDisplayHandle,
        present_modes: &[vk::PresentModeKHR],
    ) -> vk::PresentModeKHR {
        match self {
            Self::Fifo => vk::PresentModeKHR::FIFO,
            Self::Mailbox => {
                if present_modes.contains(&vk::PresentModeKHR::MAILBOX) {
                    vk::PresentModeKHR::MAILBOX
                } else {
                    vk::PresentModeKHR::FIFO
                }
            }
            Self::Auto => match display_handle {
                // Screen-capture tooling (OBS/PipeWire on X11) is more reliable with
                // compositor-friendly FIFO, so prefer it there.
                RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_) => vk::PresentModeKHR::FIFO,
                _ if present_modes.contains(&vk::PresentModeKHR::MAILBOX) => {
                    vk::PresentModeKHR::MAILBOX
                }
                _ => vk::PresentModeKHR::FIFO,
            },
        }
    }
}

/// Vulkan rendering context with swapchain.
pub struct RenderContext {
    pub entry: ash::Entry,
    pub instance: ash::Instance,
    pub surface_loader: ash::khr::surface::Instance,
    pub surface: vk::SurfaceKHR,
    pub physical_device: vk::PhysicalDevice,
    pub device: ash::Device,
    pub graphics_queue: vk::Queue,
    pub graphics_queue_family: u32,
    pub allocator: Option<Arc<Mutex<Allocator>>>,

    // Swapchain
    pub swapchain_loader: ash::khr::swapchain::Device,
    pub swapchain: vk::SwapchainKHR,
    pub swapchain_images: Vec<vk::Image>,
    pub swapchain_image_views: Vec<vk::ImageView>,
    pub swapchain_format: vk::Format,
    pub swapchain_color_space: vk::ColorSpaceKHR,
    pub swapchain_extent: vk::Extent2D,
    pub present_mode: vk::PresentModeKHR,

    // Render pass
    pub render_pass: vk::RenderPass,
    pub framebuffers: Vec<vk::Framebuffer>,

    // Command resources
    pub command_pool: vk::CommandPool,
    pub command_buffers: Vec<vk::CommandBuffer>,

    // Synchronization
    pub image_available_semaphores: Vec<vk::Semaphore>,
    pub render_finished_semaphores: Vec<vk::Semaphore>,
    pub in_flight_fences: Vec<vk::Fence>,
    pub current_frame: usize,
    pub frames_in_flight: usize,
}

impl RenderContext {
    pub fn new(window: &Window, present_mode_preference: PresentModePreference) -> Result<Self> {
        let entry = unsafe { ash::Entry::load()? };

        // Create instance with surface extensions
        let app_name = c"FluidSim Renderer";
        let engine_name = c"FluidSim Engine";

        let app_info = vk::ApplicationInfo::default()
            .application_name(app_name)
            .application_version(vk::make_api_version(0, 1, 0, 0))
            .engine_name(engine_name)
            .engine_version(vk::make_api_version(0, 1, 0, 0))
            .api_version(vk::API_VERSION_1_2);

        // Surface extensions for the current windowing backend. ash-window
        // returns the right set per platform: X11/Wayland on Linux, and the
        // Metal surface extension on macOS.
        let display_handle = window
            .display_handle()
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let raw_display_handle = display_handle.as_raw();
        let raw_window_handle = window
            .window_handle()
            .map_err(|e| anyhow::anyhow!("{}", e))?
            .as_raw();
        let mut extensions =
            ash_window::enumerate_required_extensions(raw_display_handle)?.to_vec();

        // MoltenVK (macOS) is a non-conformant portability driver that the
        // loader only enumerates when we opt in. On native Vulkan drivers the
        // extension is absent, so this collapses to the previous behavior.
        let available_instance_exts =
            unsafe { entry.enumerate_instance_extension_properties(None)? };
        let mut instance_flags = vk::InstanceCreateFlags::empty();
        if available_instance_exts.iter().any(|ext| {
            ext.extension_name_as_c_str()
                .is_ok_and(|name| name == ash::khr::portability_enumeration::NAME)
        }) {
            extensions.push(ash::khr::portability_enumeration::NAME.as_ptr());
            instance_flags |= vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR;
        }

        let instance_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .flags(instance_flags)
            .enabled_extension_names(&extensions);

        let instance = unsafe { entry.create_instance(&instance_info, None)? };
        let surface_loader = ash::khr::surface::Instance::new(&entry, &instance);

        // Create the surface. ash-window handles the per-platform details,
        // including building a CAMetalLayer-backed Metal surface on macOS.
        let surface = unsafe {
            ash_window::create_surface(
                &entry,
                &instance,
                raw_display_handle,
                raw_window_handle,
                None,
            )?
        };

        // Select physical device
        let physical_devices = unsafe { instance.enumerate_physical_devices()? };
        if physical_devices.is_empty() {
            bail!("No Vulkan-capable GPU found");
        }

        let physical_device = physical_devices
            .into_iter()
            .find(|&pd| {
                // Check for swapchain support
                let extensions = unsafe {
                    instance
                        .enumerate_device_extension_properties(pd)
                        .unwrap_or_default()
                };
                extensions.iter().any(|ext| {
                    let name = unsafe { CStr::from_ptr(ext.extension_name.as_ptr()) };
                    name == ash::khr::swapchain::NAME
                })
            })
            .context("No GPU with swapchain support found")?;

        // Find graphics queue family with present support
        let queue_families =
            unsafe { instance.get_physical_device_queue_family_properties(physical_device) };
        let graphics_queue_family = queue_families
            .iter()
            .enumerate()
            .position(|(i, qf)| {
                qf.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                    && qf.queue_flags.contains(vk::QueueFlags::COMPUTE)
                    && unsafe {
                        surface_loader
                            .get_physical_device_surface_support(physical_device, i as u32, surface)
                            .unwrap_or(false)
                    }
            })
            .context("No graphics+compute queue with present support")?
            as u32;

        // Create logical device
        let queue_priority = [1.0f32];
        let queue_info = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(graphics_queue_family)
            .queue_priorities(&queue_priority);

        let mut device_extensions = vec![ash::khr::swapchain::NAME.as_ptr()];

        // Devices that advertise VK_KHR_portability_subset (MoltenVK) must have
        // it enabled; native Vulkan drivers never expose it.
        let available_device_exts = unsafe {
            instance
                .enumerate_device_extension_properties(physical_device)
                .unwrap_or_default()
        };
        if available_device_exts.iter().any(|ext| {
            ext.extension_name_as_c_str()
                .is_ok_and(|name| name == ash::khr::portability_subset::NAME)
        }) {
            device_extensions.push(ash::khr::portability_subset::NAME.as_ptr());
        }

        let device_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(std::slice::from_ref(&queue_info))
            .enabled_extension_names(&device_extensions);

        let device = unsafe { instance.create_device(physical_device, &device_info, None)? };
        let graphics_queue = unsafe { device.get_device_queue(graphics_queue_family, 0) };

        // Create memory allocator
        let allocator = Allocator::new(&gpu_allocator::vulkan::AllocatorCreateDesc {
            instance: instance.clone(),
            device: device.clone(),
            physical_device,
            debug_settings: Default::default(),
            buffer_device_address: false,
            allocation_sizes: Default::default(),
        })?;
        let allocator = Arc::new(Mutex::new(allocator));

        // Create swapchain
        let swapchain_loader = ash::khr::swapchain::Device::new(&instance, &device);

        let surface_caps = unsafe {
            surface_loader.get_physical_device_surface_capabilities(physical_device, surface)?
        };
        let surface_formats = unsafe {
            surface_loader.get_physical_device_surface_formats(physical_device, surface)?
        };
        let present_modes = unsafe {
            surface_loader.get_physical_device_surface_present_modes(physical_device, surface)?
        };

        // Choose format (prefer SRGB)
        let format = surface_formats
            .iter()
            .find(|f| f.format == vk::Format::B8G8R8A8_SRGB)
            .unwrap_or(&surface_formats[0]);

        // Choose present mode (prefer mailbox for low latency)
        let present_mode = present_mode_preference.select(raw_display_handle, &present_modes);

        // Choose extent: honor the surface's fixed extent, otherwise clamp the
        // window's pixel size into the surface's allowed range.
        let extent = if surface_caps.current_extent.width != u32::MAX {
            surface_caps.current_extent
        } else {
            let size = window.inner_size();
            clamp_extent(&surface_caps, size.width, size.height)
        };

        let image_count = select_image_count(&surface_caps);

        let swapchain_info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(image_count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(surface_caps.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true);

        let swapchain = unsafe { swapchain_loader.create_swapchain(&swapchain_info, None)? };
        let swapchain_images = unsafe { swapchain_loader.get_swapchain_images(swapchain)? };

        let swapchain_image_views =
            create_swapchain_image_views(&device, &swapchain_images, format.format)?;

        // Create render pass
        let color_attachment = vk::AttachmentDescription::default()
            .format(format.format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);

        let color_attachment_ref = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(std::slice::from_ref(&color_attachment_ref));

        let dependency = vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags::empty())
            .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE);

        let render_pass_info = vk::RenderPassCreateInfo::default()
            .attachments(std::slice::from_ref(&color_attachment))
            .subpasses(std::slice::from_ref(&subpass))
            .dependencies(std::slice::from_ref(&dependency));

        let render_pass = unsafe { device.create_render_pass(&render_pass_info, None)? };

        let framebuffers =
            create_framebuffers(&device, render_pass, &swapchain_image_views, extent)?;

        // Create command pool and buffers
        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(graphics_queue_family)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        let command_pool = unsafe { device.create_command_pool(&pool_info, None)? };

        let frames_in_flight = 2;
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(frames_in_flight as u32);
        let command_buffers = unsafe { device.allocate_command_buffers(&alloc_info)? };

        // Create synchronization objects
        let semaphore_info = vk::SemaphoreCreateInfo::default();
        let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);

        let mut image_available_semaphores = Vec::with_capacity(frames_in_flight);
        let mut render_finished_semaphores = Vec::with_capacity(frames_in_flight);
        let mut in_flight_fences = Vec::with_capacity(frames_in_flight);

        for _ in 0..frames_in_flight {
            image_available_semaphores
                .push(unsafe { device.create_semaphore(&semaphore_info, None)? });
            render_finished_semaphores
                .push(unsafe { device.create_semaphore(&semaphore_info, None)? });
            in_flight_fences.push(unsafe { device.create_fence(&fence_info, None)? });
        }

        Ok(Self {
            entry,
            instance,
            surface_loader,
            surface,
            physical_device,
            device,
            graphics_queue,
            graphics_queue_family,
            allocator: Some(allocator),
            swapchain_loader,
            swapchain,
            swapchain_images,
            swapchain_image_views,
            swapchain_format: format.format,
            swapchain_color_space: format.color_space,
            swapchain_extent: extent,
            present_mode,
            render_pass,
            framebuffers,
            command_pool,
            command_buffers,
            image_available_semaphores,
            render_finished_semaphores,
            in_flight_fences,
            current_frame: 0,
            frames_in_flight,
        })
    }

    /// Recreate swapchain after resize.
    pub fn recreate_swapchain(&mut self, width: u32, height: u32) -> Result<()> {
        unsafe { self.device.device_wait_idle()? };

        // Clean up old swapchain resources
        for &fb in &self.framebuffers {
            unsafe { self.device.destroy_framebuffer(fb, None) };
        }
        for &view in &self.swapchain_image_views {
            unsafe { self.device.destroy_image_view(view, None) };
        }
        let old_swapchain = self.swapchain;

        // Get surface capabilities
        let surface_caps = unsafe {
            self.surface_loader
                .get_physical_device_surface_capabilities(self.physical_device, self.surface)?
        };

        let extent = clamp_extent(&surface_caps, width, height);
        let image_count = select_image_count(&surface_caps);

        let swapchain_info = vk::SwapchainCreateInfoKHR::default()
            .surface(self.surface)
            .min_image_count(image_count)
            .image_format(self.swapchain_format)
            .image_color_space(self.swapchain_color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(surface_caps.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(self.present_mode)
            .clipped(true)
            .old_swapchain(old_swapchain);

        self.swapchain = unsafe {
            self.swapchain_loader
                .create_swapchain(&swapchain_info, None)?
        };

        unsafe { self.swapchain_loader.destroy_swapchain(old_swapchain, None) };

        self.swapchain_images =
            unsafe { self.swapchain_loader.get_swapchain_images(self.swapchain)? };
        self.swapchain_extent = extent;

        self.swapchain_image_views = create_swapchain_image_views(
            &self.device,
            &self.swapchain_images,
            self.swapchain_format,
        )?;
        self.framebuffers = create_framebuffers(
            &self.device,
            self.render_pass,
            &self.swapchain_image_views,
            extent,
        )?;

        Ok(())
    }

    /// Begin a frame, returning the image index to render to.
    pub fn begin_frame(&mut self) -> Result<Option<u32>> {
        unsafe {
            self.device.wait_for_fences(
                &[self.in_flight_fences[self.current_frame]],
                true,
                u64::MAX,
            )?;
        }

        let result = unsafe {
            self.swapchain_loader.acquire_next_image(
                self.swapchain,
                u64::MAX,
                self.image_available_semaphores[self.current_frame],
                vk::Fence::null(),
            )
        };

        match result {
            Ok((index, false)) => {
                unsafe {
                    self.device
                        .reset_fences(&[self.in_flight_fences[self.current_frame]])?;
                }
                Ok(Some(index))
            }
            Ok((_, true)) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                Ok(None) // Needs resize
            }
            Err(e) => bail!("Failed to acquire swapchain image: {:?}", e),
        }
    }

    /// End a frame and present.
    pub fn end_frame(&mut self, image_index: u32) -> Result<bool> {
        let wait_semaphores = [self.image_available_semaphores[self.current_frame]];
        let signal_semaphores = [self.render_finished_semaphores[self.current_frame]];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let command_buffers = [self.command_buffers[self.current_frame]];

        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&command_buffers)
            .signal_semaphores(&signal_semaphores);

        unsafe {
            self.device.queue_submit(
                self.graphics_queue,
                &[submit_info],
                self.in_flight_fences[self.current_frame],
            )?;
        }

        let swapchains = [self.swapchain];
        let image_indices = [image_index];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal_semaphores)
            .swapchains(&swapchains)
            .image_indices(&image_indices);

        let result = unsafe {
            self.swapchain_loader
                .queue_present(self.graphics_queue, &present_info)
        };

        self.current_frame = (self.current_frame + 1) % self.frames_in_flight;

        match result {
            Ok(false) => Ok(true),
            Ok(true) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => Ok(false),
            Err(e) => bail!("Failed to present: {:?}", e),
        }
    }

    pub fn current_command_buffer(&self) -> vk::CommandBuffer {
        self.command_buffers[self.current_frame]
    }

    pub fn current_frame_index(&self) -> usize {
        self.current_frame
    }

    pub fn is_frame_complete(&self, frame_index: usize) -> Result<bool> {
        unsafe {
            self.device
                .get_fence_status(self.in_flight_fences[frame_index])
        }
        .with_context(|| format!("Failed to query fence status for frame {frame_index}"))
    }

    pub fn shared_gpu_context(&self) -> SharedGpuContext {
        SharedGpuContext {
            instance: self.instance.clone(),
            physical_device: self.physical_device,
            device: self.device.clone(),
            queue: self.graphics_queue,
            queue_family: self.graphics_queue_family,
            allocator: self.allocator.as_ref().unwrap().clone(),
        }
    }
}

impl Drop for RenderContext {
    fn drop(&mut self) {
        log::info!("RenderContext::drop - starting");
        unsafe {
            self.device.device_wait_idle().ok();
            log::info!("RenderContext::drop - device idle");

            for &sem in &self.image_available_semaphores {
                self.device.destroy_semaphore(sem, None);
            }
            for &sem in &self.render_finished_semaphores {
                self.device.destroy_semaphore(sem, None);
            }
            for &fence in &self.in_flight_fences {
                self.device.destroy_fence(fence, None);
            }

            self.device.destroy_command_pool(self.command_pool, None);

            for &fb in &self.framebuffers {
                self.device.destroy_framebuffer(fb, None);
            }
            for &view in &self.swapchain_image_views {
                self.device.destroy_image_view(view, None);
            }
            self.device.destroy_render_pass(self.render_pass, None);
            self.swapchain_loader
                .destroy_swapchain(self.swapchain, None);
            self.surface_loader.destroy_surface(self.surface, None);
            log::info!("RenderContext::drop - vulkan objects destroyed");

            // Drop the allocator BEFORE destroying the device.
            // gpu_allocator::Allocator::drop calls vkFreeMemory, which requires a live device.
            drop(self.allocator.take());
            log::info!("RenderContext::drop - allocator dropped");

            self.device.destroy_device(None);
            log::info!("RenderContext::drop - device destroyed");
            self.instance.destroy_instance(None);
            log::info!("RenderContext::drop - instance destroyed");
        }
    }
}

/// Clamp a desired width/height into the surface's supported extent range.
fn clamp_extent(caps: &vk::SurfaceCapabilitiesKHR, width: u32, height: u32) -> vk::Extent2D {
    vk::Extent2D {
        width: width.clamp(caps.min_image_extent.width, caps.max_image_extent.width),
        height: height.clamp(caps.min_image_extent.height, caps.max_image_extent.height),
    }
}

/// Pick a swapchain image count: one above the minimum, capped by the maximum
/// (where a maximum of 0 means "no upper bound").
fn select_image_count(caps: &vk::SurfaceCapabilitiesKHR) -> u32 {
    let max = if caps.max_image_count > 0 {
        caps.max_image_count
    } else {
        u32::MAX
    };
    (caps.min_image_count + 1).min(max)
}

/// Create a 2D color image view for each swapchain image.
fn create_swapchain_image_views(
    device: &ash::Device,
    images: &[vk::Image],
    format: vk::Format,
) -> Result<Vec<vk::ImageView>, vk::Result> {
    images
        .iter()
        .map(|&image| {
            let view_info = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(format)
                .components(vk::ComponentMapping {
                    r: vk::ComponentSwizzle::IDENTITY,
                    g: vk::ComponentSwizzle::IDENTITY,
                    b: vk::ComponentSwizzle::IDENTITY,
                    a: vk::ComponentSwizzle::IDENTITY,
                })
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });
            unsafe { device.create_image_view(&view_info, None) }
        })
        .collect()
}

/// Create one framebuffer per image view, all bound to `render_pass`.
fn create_framebuffers(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    image_views: &[vk::ImageView],
    extent: vk::Extent2D,
) -> Result<Vec<vk::Framebuffer>, vk::Result> {
    image_views
        .iter()
        .map(|&view| {
            let attachments = [view];
            let fb_info = vk::FramebufferCreateInfo::default()
                .render_pass(render_pass)
                .attachments(&attachments)
                .width(extent.width)
                .height(extent.height)
                .layers(1);
            unsafe { device.create_framebuffer(&fb_info, None) }
        })
        .collect()
}

