//! The GPU side. The 2D picture is one full-frame shader fed a small texture
//! of bin levels. The 3D view adds a map of the picture, drawn first each
//! frame, that the 3D shader and the storm's raindrops read heights from.
//! With bloom on, the picture is drawn to a texture of its own, its bright
//! parts are blurred at two sizes, and the two are put together on screen.

use crate::analysis::N_BINS;
use crate::nowplaying::{COVER_SIZE, Cover};
use eframe::egui_wgpu::{self, wgpu};
use std::sync::Arc;

/// Rows of the data texture: x-axis level, y-axis level, stereo position,
/// out-of-step amount, nine rows of widening maxima for each level row, then
/// one row of single-bin maxima for each level row.
pub const ROWS: u32 = 4 + 2 * MAX_ROWS as u32 + 2;
const MAX_ROWS: usize = 9;

/// Side of the square map of the picture used by the 3D view, in pixels:
/// [cross, circle]. Rings cut across the pixel grid at every angle, so the
/// circle needs the finer map to keep the tops of thin walls smooth.
#[cfg(not(target_os = "android"))]
const MAP_SIZES: [u32; 2] = [2048, 4096];
/// A phone has a smaller screen and far less graphics memory, so its maps
/// are half the size each way.
#[cfg(target_os = "android")]
const MAP_SIZES: [u32; 2] = [1024, 2048];
const MAP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Raindrops simulated at full storm strength.
pub const MAX_DROPS: u32 = 24_000;
const DROP_BYTES: u64 = 32;
/// The picture and its glow are worked on in this format, whatever the screen's.
const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Append rows giving, for each bin, the loudest level within 2, 4, 8 ... 512
/// bins of it. The 3D view uses them to know how high the surface can be
/// across a stretch of ground without sampling every point.
pub fn push_maxima(levels: &mut Vec<f32>, row: &[f32]) {
    let n = row.len();
    let at = |i: isize| i.clamp(0, n as isize - 1) as usize;
    let start = levels.len();
    for i in 0..n as isize {
        levels.push((-2..=2).map(|d| row[at(i + d)]).fold(0.0, f32::max));
    }
    for k in 0..MAX_ROWS - 1 {
        let reach = 2isize << k;
        let previous = start + k * n;
        for i in 0..n as isize {
            levels.push(levels[previous + at(i - reach)].max(levels[previous + at(i + reach)]));
        }
    }
}

/// Append a row giving the loudest level anywhere inside each bin. Levels are
/// blended across four neighbouring bins, so a bin is touched by two either side.
pub fn push_bin_maxima(levels: &mut Vec<f32>, row: &[f32]) {
    let last = row.len() - 1;
    for i in 0..row.len() {
        levels.push(row[i.saturating_sub(2)..=(i + 2).min(last)].iter().copied().fold(0.0, f32::max));
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub layout: [f32; 4],
    pub tone: [f32; 4],
    pub view: [f32; 4],
    pub circle: [f32; 4],
    pub accent: [f32; 4],
    pub extra: [f32; 4],
    pub stereo: [f32; 4],
    pub surround: [f32; 4],
    pub relief: [f32; 4],
    pub cam_eye: [f32; 4],
    pub cam_target: [f32; 4],
    /// Seconds since the last frame, number of raindrops in use, 3D material strength, how far the flight through the stars has gone.
    pub sim: [f32; 4],
    /// HDR on (1) or off (0), base and peak brightness in units of 80 nits, unused.
    pub hdr: [f32; 4],
    /// Bloom amount, cover behind the picture (2) or not (0), its brightness, treble level.
    pub post: [f32; 4],
    /// Stars: brightness (0 is off), how many, how much their sizes differ, unused.
    pub stars: [f32; 4],
    /// Flight through the stars: where it is heading (xy) and how far the stars have slid in its turns (zw).
    pub star_turn: [f32; 4],
    pub stops: [[f32; 4]; 9],
}

/// The textures bloom works in, made for one size of picture.
struct Targets {
    size: [u32; 2],
    scene: wgpu::TextureView,
    /// The glow at a quarter and a sixteenth of the size: two of each, to blur from one into the other.
    near: [wgpu::TextureView; 2],
    wide: [wgpu::TextureView; 2],
    read_scene: wgpu::BindGroup,
    read_near: [wgpu::BindGroup; 2],
    read_wide: [wgpu::BindGroup; 2],
    /// The settings and both finished glows, for the last step.
    glow: wgpu::BindGroup,
}

impl Targets {
    fn new(device: &wgpu::Device, res: &Resources, size: [u32; 2]) -> Self {
        let texture = |divide: u32| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("audiovis bloom"),
                    size: wgpu::Extent3d { width: (size[0] / divide).max(1), height: (size[1] / divide).max(1), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: SCENE_FORMAT,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let read = |view: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("audiovis bloom source"),
                layout: &res.map_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&res.clamp) },
                ],
            })
        };
        let scene = texture(1);
        let near = [texture(4), texture(4)];
        let wide = [texture(16), texture(16)];
        let glow = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("audiovis glow"),
            layout: &res.glow_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: res.post_uniforms.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&near[0]) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&wide[0]) },
            ],
        });
        Self {
            size,
            read_scene: read(&scene),
            read_near: [read(&near[0]), read(&near[1])],
            read_wide: [read(&wide[0]), read(&wide[1])],
            glow,
            scene,
            near,
            wide,
        }
    }
}

struct Resources {
    picture: wgpu::RenderPipeline,
    /// The picture drawn to a texture, and the steps that add its glow.
    scene: wgpu::RenderPipeline,
    bright: wgpu::RenderPipeline,
    shrink: wgpu::RenderPipeline,
    blur_across: wgpu::RenderPipeline,
    blur_down: wgpu::RenderPipeline,
    last: wgpu::RenderPipeline,
    map_layout: wgpu::BindGroupLayout,
    glow_layout: wgpu::BindGroupLayout,
    clamp: wgpu::Sampler,
    post_uniforms: wgpu::Buffer,
    targets: Option<Targets>,
    cover: wgpu::Texture,
    map: wgpu::RenderPipeline,
    rain_step: wgpu::ComputePipeline,
    rain_draw: wgpu::RenderPipeline,
    data: wgpu::BindGroup,
    /// Picture maps, [cross, circle]: for reading, and for drawing into.
    map_read: [wgpu::BindGroup; 2],
    drops_write: wgpu::BindGroup,
    drops_read: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    levels: wgpu::Texture,
    map_view: [wgpu::TextureView; 2],
}

pub fn init(render_state: &egui_wgpu::RenderState) {
    let device = &render_state.device;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("audiovis shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
    });

    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("audiovis uniforms"),
        size: std::mem::size_of::<Uniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let levels = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("audiovis levels"),
        size: wgpu::Extent3d { width: N_BINS as u32, height: ROWS, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let map_view = MAP_SIZES.map(|size| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("audiovis picture map"),
                size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: MAP_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default())
    });
    let post_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("audiovis bloom shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("post.wgsl").into()),
    });
    let post_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("audiovis bloom settings"),
        size: 32,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    // The cover starts black, which is also what shows when a track has none.
    let cover = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("audiovis cover"),
        size: wgpu::Extent3d { width: COVER_SIZE, height: COVER_SIZE, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let clamp = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("audiovis smooth sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    // Mirrored repeat: beyond its edges the cross picture continues as mirror images.
    let map_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("audiovis picture map sampler"),
        address_mode_u: wgpu::AddressMode::MirrorRepeat,
        address_mode_v: wgpu::AddressMode::MirrorRepeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    // Every drop starts unused; the simulation brings them into play.
    let unused: Vec<f32> = (0..MAX_DROPS).flat_map(|_| [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0]).collect();
    let drops = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("audiovis raindrops"),
        size: MAX_DROPS as u64 * DROP_BYTES,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    render_state.queue.write_buffer(&drops, 0, bytemuck::cast_slice(&unused));

    let all_stages = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE;
    let data_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("audiovis data layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: all_stages,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let glow_texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let glow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("audiovis glow layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            glow_texture(1),
            glow_texture(2),
        ],
    });
    let map_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("audiovis picture map layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: all_stages,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: all_stages,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let drops_layout = |read_only: bool, visibility| {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("audiovis raindrops layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        })
    };
    let drops_write_layout = drops_layout(false, wgpu::ShaderStages::COMPUTE);
    let drops_read_layout = drops_layout(true, wgpu::ShaderStages::VERTEX);

    let data = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("audiovis data"),
        layout: &data_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(
                    &levels.create_view(&wgpu::TextureViewDescriptor::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&cover.create_view(&wgpu::TextureViewDescriptor::default())),
            },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&clamp) },
        ],
    });
    let map_read = [0, 1].map(|i| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("audiovis picture map"),
            layout: &map_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&map_view[i]) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&map_sampler) },
            ],
        })
    });
    let drops_group = |layout: &wgpu::BindGroupLayout| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("audiovis raindrops"),
            layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: drops.as_entire_binding() }],
        })
    };
    let drops_write = drops_group(&drops_write_layout);
    let drops_read = drops_group(&drops_read_layout);

    let layout_of = |groups: &[Option<&wgpu::BindGroupLayout>]| {
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("audiovis pipeline layout"),
            bind_group_layouts: groups,
            immediate_size: 0,
        })
    };
    let full_frame_of = |module: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout, entry: &str, target: wgpu::ColorTargetState| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(entry),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                targets: &[Some(target)],
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    let full_frame = |layout: &wgpu::PipelineLayout, entry: &str, target: wgpu::ColorTargetState| full_frame_of(&shader, layout, entry, target);
    let scene = full_frame(&layout_of(&[Some(&data_layout), Some(&map_layout)]), "fs_scene", SCENE_FORMAT.into());
    let glow_step = |entry: &str| full_frame_of(&post_shader, &layout_of(&[Some(&map_layout)]), entry, SCENE_FORMAT.into());
    let (bright, shrink, blur_across, blur_down) = (glow_step("fs_bright"), glow_step("fs_shrink"), glow_step("fs_blur_across"), glow_step("fs_blur_down"));
    let last = full_frame_of(
        &post_shader,
        &layout_of(&[Some(&map_layout), Some(&glow_layout)]),
        "fs_final",
        render_state.target_format.into(),
    );
    let picture = full_frame(
        &layout_of(&[Some(&data_layout), Some(&map_layout)]),
        "fs",
        render_state.target_format.into(),
    );
    let map = full_frame(&layout_of(&[Some(&data_layout)]), "fs_map", MAP_FORMAT.into());

    let rain_step = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("audiovis rain step"),
        layout: Some(&layout_of(&[Some(&data_layout), Some(&map_layout), Some(&drops_write_layout)])),
        module: &shader,
        entry_point: Some("cs_rain"),
        compilation_options: Default::default(),
        cache: None,
    });
    // Drops add their light to whatever is behind them.
    let add = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    };
    let rain_draw = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("audiovis rain draw"),
        layout: Some(&layout_of(&[Some(&data_layout), Some(&map_layout), None, Some(&drops_read_layout)])),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_drop"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_drop"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: render_state.target_format,
                blend: Some(wgpu::BlendState { color: add, alpha: add }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });

    render_state.renderer.write().callback_resources.insert(Resources {
        picture,
        scene,
        bright,
        shrink,
        blur_across,
        blur_down,
        last,
        map_layout,
        glow_layout,
        clamp,
        post_uniforms,
        targets: None,
        cover,
        map,
        rain_step,
        rain_draw,
        data,
        map_read,
        drops_write,
        drops_read,
        uniforms,
        levels,
        map_view,
    });
}

/// One frame's worth of data for the shader.
pub struct Frame {
    pub uniforms: Uniforms,
    /// `ROWS` rows of `N_BINS` values each.
    pub levels: Vec<f32>,
    /// Size of the picture on screen, in pixels.
    pub size: [u32; 2],
    /// A new cover to show, when the track's has changed.
    pub cover: Option<Arc<Cover>>,
}

impl Frame {
    fn is_3d(&self) -> bool {
        self.uniforms.relief[0] > 0.5
    }

    /// Which picture map this frame uses: 0 cross, 1 circle.
    fn map(&self) -> usize {
        (self.uniforms.circle[0] > 0.5) as usize
    }

    fn bloom(&self) -> bool {
        self.uniforms.post[0] > 0.001
    }

    fn raining(&self) -> bool {
        self.is_3d() && self.uniforms.sim[1] >= 1.0
    }
}

impl egui_wgpu::CallbackTrait for Frame {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let res: &mut Resources = resources.get_mut().unwrap();
        queue.write_buffer(&res.uniforms, 0, bytemuck::bytes_of(&self.uniforms));
        if let Some(cover) = self.cover.as_ref().filter(|c| c.rgba.len() == (4 * COVER_SIZE * COVER_SIZE) as usize) {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &res.cover,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &cover.rgba,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * COVER_SIZE), rows_per_image: Some(COVER_SIZE) },
                wgpu::Extent3d { width: COVER_SIZE, height: COVER_SIZE, depth_or_array_layers: 1 },
            );
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &res.levels,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&self.levels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * N_BINS as u32),
                rows_per_image: Some(ROWS),
            },
            wgpu::Extent3d { width: N_BINS as u32, height: ROWS, depth_or_array_layers: 1 },
        );

        if self.is_3d() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("audiovis picture map"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &res.map_view[self.map()],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&res.map);
            pass.set_bind_group(0, &res.data, &[]);
            pass.draw(0..3, 0..1);
        }
        if self.raining() {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("audiovis rain step"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&res.rain_step);
            pass.set_bind_group(0, &res.data, &[]);
            pass.set_bind_group(1, &res.map_read[self.map()], &[]);
            pass.set_bind_group(2, &res.drops_write, &[]);
            pass.dispatch_workgroups(MAX_DROPS.div_ceil(64), 1, 1);
        }
        if self.bloom() {
            let size = [self.size[0].max(16), self.size[1].max(16)];
            if res.targets.as_ref().is_none_or(|t| t.size != size) {
                res.targets = Some(Targets::new(device, res, size));
            }
            queue.write_buffer(&res.post_uniforms, 0, bytemuck::bytes_of(&[self.uniforms.hdr, [self.uniforms.post[0], 0.0, 0.0, 0.0]]));
            let targets = res.targets.as_ref().unwrap();
            let mut step = |pipeline: &wgpu::RenderPipeline, into: &wgpu::TextureView, groups: [&wgpu::BindGroup; 2], count: usize| {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("audiovis bloom"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: into,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(pipeline);
                for (i, group) in groups.into_iter().take(count).enumerate() {
                    pass.set_bind_group(i as u32, group, &[]);
                }
                pass.draw(0..3, 0..1);
            };
            // The picture, then its bright parts shrunk and blurred, then those shrunk and blurred again.
            step(&res.scene, &targets.scene, [&res.data, &res.map_read[self.map()]], 2);
            step(&res.bright, &targets.near[0], [&targets.read_scene, &targets.read_scene], 1);
            step(&res.blur_across, &targets.near[1], [&targets.read_near[0], &targets.read_near[0]], 1);
            step(&res.blur_down, &targets.near[0], [&targets.read_near[1], &targets.read_near[1]], 1);
            step(&res.shrink, &targets.wide[0], [&targets.read_near[0], &targets.read_near[0]], 1);
            step(&res.blur_across, &targets.wide[1], [&targets.read_wide[0], &targets.read_wide[0]], 1);
            step(&res.blur_down, &targets.wide[0], [&targets.read_wide[1], &targets.read_wide[1]], 1);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: eframe::egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let res: &Resources = resources.get().unwrap();
        match res.targets.as_ref().filter(|_| self.bloom()) {
            Some(targets) => {
                render_pass.set_pipeline(&res.last);
                render_pass.set_bind_group(0, &targets.read_scene, &[]);
                render_pass.set_bind_group(1, &targets.glow, &[]);
                render_pass.draw(0..3, 0..1);
                // The rain's own bindings start from the picture's.
                render_pass.set_bind_group(0, &res.data, &[]);
                render_pass.set_bind_group(1, &res.map_read[self.map()], &[]);
            }
            None => {
                render_pass.set_pipeline(&res.picture);
                render_pass.set_bind_group(0, &res.data, &[]);
                render_pass.set_bind_group(1, &res.map_read[self.map()], &[]);
                render_pass.draw(0..3, 0..1);
            }
        }
        if self.raining() {
            render_pass.set_pipeline(&res.rain_draw);
            render_pass.set_bind_group(3, &res.drops_read, &[]);
            render_pass.draw(0..6, 0..MAX_DROPS);
        }
    }
}
