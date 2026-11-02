//! Putting the picture on screen through the graphics card: DirectX 12, Vulkan or OpenGL.
//!
//! Beacon composes each frame on the CPU (game, map, menu overlay) exactly as before; this
//! only changes how the finished frame reaches the screen. Drawing through a swapchain is
//! what makes Beacon a game to the tools built for games: OBS's Game Capture hooks a
//! DirectX, Vulkan or OpenGL swapchain and cannot see a window painted with ordinary Windows
//! drawing, which is what the CPU path ([`softbuffer`]) does.
//!
//! The frame is uploaded at the window's size, one texel per pixel, and drawn by a single
//! triangle covering the screen that reads it with `textureLoad`: no sampling, so no blur and
//! no colour shift. The texture is BGRA, which is the memory order of the `0x00RRGGBB` pixels
//! the rest of Beacon works in, so the upload is a plain copy.

use std::sync::Arc;

use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

/// A swapchain on the window and what it takes to draw one texture into it.
pub struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// The uploaded frame, and the bind group reading it; rebuilt when the window resizes.
    frame: Option<(wgpu::Texture, wgpu::BindGroup, u32, u32)>,
    /// For the log line, and for a player asking which one is in use.
    pub backend: String,
}

const SHADER: &str = r#"
@group(0) @binding(0) var frame: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // One triangle that covers the screen: (-1,-1), (3,-1), (-1,3).
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(frame, vec2<i32>(p.xy), 0);
    return vec4<f32>(c.rgb, 1.0);
}
"#;

impl Gpu {
    /// Opens a swapchain on `window`, trying each backend this platform has in turn: on
    /// Windows DirectX 12 first, since that is what capture tools and AMD, NVIDIA and Intel
    /// drivers all handle best, then Vulkan, then OpenGL. `None` if none of them start, and
    /// the caller draws on the CPU instead. Each failure is logged, so a machine where the
    /// card is not used says why.
    pub fn new(event_loop: &ActiveEventLoop, window: &Arc<Window>) -> Option<Gpu> {
        let order: &[wgpu::Backends] = if cfg!(windows) {
            &[wgpu::Backends::DX12, wgpu::Backends::VULKAN, wgpu::Backends::GL]
        } else {
            &[wgpu::Backends::VULKAN, wgpu::Backends::GL]
        };
        for &backends in order {
            match Self::with_backend(event_loop, window, backends) {
                Ok(gpu) => {
                    eprintln!("video: drawing with {}", gpu.backend);
                    return Some(gpu);
                }
                Err(e) => eprintln!("video: {backends:?} unavailable: {e}"),
            }
        }
        None
    }

    fn with_backend(
        event_loop: &ActiveEventLoop,
        window: &Arc<Window>,
        backends: wgpu::Backends,
    ) -> Result<Gpu, String> {
        let mut desc =
            wgpu::InstanceDescriptor::new_with_display_handle(Box::new(event_loop.owned_display_handle()));
        desc.backends = backends;
        let instance = wgpu::Instance::new(desc);
        let surface = instance
            .create_surface(Arc::clone(window))
            .map_err(|e| e.to_string())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("beacon"),
            // The least any card offers: a 2-D texture and a triangle is all this needs.
            required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                .using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;

        let caps = surface.get_capabilities(&adapter);
        // A format without sRGB encoding, so the frame's colours reach the screen as they are;
        // an sRGB target would encode them a second time and wash the picture out.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .ok_or("the surface offers no formats")?;
        // Mailbox shows the newest frame without waiting for the screen, so presenting never
        // holds up the emulator, which the audio paces; Fifo (vsync) is the guaranteed one.
        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            desired_maximum_frame_latency: 1,
            alpha_mode: caps
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: vec![],
            // sRGB as every SDR screen, so the colours are the game's own.
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("beacon"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("beacon"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("beacon"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("beacon"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Ok(Gpu {
            surface,
            device,
            queue,
            config,
            pipeline,
            layout,
            frame: None,
            backend: format!("{:?} on {}", info.backend, info.name),
        })
    }

    /// Shows `pixels`, `width` by `height` `0x00RRGGBB`, filling the window. Returns false if
    /// the swapchain could not be used at all, so the caller can fall back to the CPU; a
    /// frame skipped because the window is hidden or mid-resize is not a failure.
    pub fn present(&mut self, pixels: &[u32], width: u32, height: u32) -> bool {
        if width == 0 || height == 0 || pixels.len() < (width * height) as usize {
            return true;
        }
        if self.config.width != width || self.config.height != height {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
        }
        let target = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return true;
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return true;
            }
            _ => return false,
        };

        if !matches!(self.frame, Some((_, _, w, h)) if w == width && h == height) {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("frame"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("frame"),
                layout: &self.layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                }],
            });
            self.frame = Some((texture, group, width, height));
        }
        let Some((texture, group, _, _)) = self.frame.as_ref() else {
            return true;
        };

        let pixels = &pixels[..(width * height) as usize];
        // The pixels as bytes, without copying: a u32 is four bytes, and on a little-endian
        // machine `0x00RRGGBB` lies in memory as B, G, R, 0, which is the texture's BGRA.
        #[cfg(target_endian = "little")]
        // SAFETY: u8 has no alignment requirement and every bit pattern is valid, and the
        // length covers exactly the u32 slice's bytes, which it borrows.
        let bytes: &[u8] =
            unsafe { std::slice::from_raw_parts(pixels.as_ptr().cast::<u8>(), pixels.len() * 4) };
        #[cfg(target_endian = "big")]
        let bytes: &[u8] = &pixels.iter().flat_map(|p| p.to_le_bytes()).collect::<Vec<u8>>();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let view = target.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(target);
        true
    }
}
