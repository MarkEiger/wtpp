//! GPU renderer for a terminal grid.
//!
//! The whole viewport is drawn as instanced quads in a single draw call:
//! cell backgrounds and selection first, then glyphs sampled from a texture
//! atlas, then decorations and the cursor.

mod atlas;
pub mod theme;

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use std::rc::Rc;

use cosmic_text::{Attrs, Buffer, CacheKey, Family, Metrics, Shaping, Style, SwashCache, SwashContent, Weight};
use wtpp_session::alacritty_terminal::Term;
use wtpp_session::alacritty_terminal::event::EventListener;
use wtpp_session::alacritty_terminal::term::cell::Flags;
use wtpp_session::alacritty_terminal::index::Point;
use wtpp_session::alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};

use atlas::{ATLAS_SIZE, Atlas};
pub use cosmic_text::FontSystem;

/// The system font database, shared by every renderer (one per font
/// face/size in use).
pub type SharedFonts = std::rc::Rc<std::cell::RefCell<FontSystem>>;
pub use theme::Theme;

/// Fonts tried in order; the first one installed wins. Cascadia Mono is
/// Windows Terminal's default.
const FALLBACK_FONTS: &[&str] = &[
    "Cascadia Mono",
    "Cascadia Code",
    "JetBrains Mono",
    "Menlo",
    "Consolas",
    "DejaVu Sans Mono",
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMetrics {
    /// Cell width in physical pixels.
    pub width: f32,
    /// Cell (line) height in physical pixels.
    pub height: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    pos: [f32; 2],
    size: [f32; 2],
    uv_pos: [f32; 2],
    color: [f32; 4],
    kind: u32,
}

/// A cell's text for shaping.
struct TextCell {
    col: usize,
    /// 2 for wide characters.
    width: usize,
    text: String,
    fg: Rgb,
    bold: bool,
    italic: bool,
    /// Shape on its own (the cursor cell).
    breaks_run: bool,
}

/// Find matches other than the current one (which is the selection).
const MATCH_HIGHLIGHT: Rgb = Rgb { r: 0xF9, g: 0xD8, b: 0x49 };

const KIND_SOLID: u32 = 0;
const KIND_MASK: u32 = 1;
const KIND_COLOR: u32 = 2;

impl Instance {
    fn solid(x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) -> Instance {
        Instance { pos: [x, y], size: [w, h], uv_pos: [0.0; 2], color, kind: KIND_SOLID }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    screen: [f32; 2],
    atlas: [f32; 2],
}

#[derive(Clone, Copy)]
struct Glyph {
    atlas_pos: [u32; 2],
    size: [u32; 2],
    /// Offset of the bitmap's top-left from the cell's top-left.
    offset: [i32; 2],
    color: bool,
}


pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals: wgpu::Buffer,
    instances: wgpu::Buffer,
    instance_capacity: usize,
    atlas: Atlas,
    fonts: SharedFonts,
    swash: SwashCache,
    family: String,
    font_size: f32,
    weight: u16,
    cell_height: f32,
    cell: CellMetrics,
    /// Rasterized glyphs by font, glyph id, size and subpixel position.
    glyphs: HashMap<CacheKey, Option<Glyph>>,
    /// Shaped text runs: (text, bold, italic) → glyphs.
    runs: HashMap<(String, bool, bool), Rc<[ShapedGlyph]>>,
    ligatures: bool,
    /// A glyph didn't fit: start the atlas over on the next frame.
    atlas_full: bool,
    /// Turns the premultiplied drawing of a see-through frame into the
    /// straight alpha Slint expects of a texture.
    resolve_pipeline: wgpu::RenderPipeline,
    resolve_layout: wgpu::BindGroupLayout,
    /// Premultiplied drawing targets of see-through frames, by size (panes of
    /// different sizes share a renderer).
    scratch: HashMap<(u32, u32), (wgpu::Texture, wgpu::BindGroup)>,
}

/// Scratch textures kept before the cache starts over.
const MAX_SCRATCH: usize = 8;

/// Whether a cell's background gets its own quad. In a see-through frame
/// only the default background is left out, so a colour the program chose
/// still covers the image even when it equals the scheme's background (as
/// in Windows Terminal); an opaque frame was already cleared to that colour.
fn paints_background(cell_bg: Color, inverse: bool, is_block_cursor: bool, resolved: Rgb, theme_bg: Rgb, transparent: bool) -> bool {
    if transparent {
        inverse || is_block_cursor || !matches!(cell_bg, Color::Named(NamedColor::Background))
    } else {
        resolved != theme_bg
    }
}

/// One glyph of a shaped run, positioned relative to the cell it starts in.
#[derive(Clone, Copy)]
struct ShapedGlyph {
    /// Byte offset in the run's text where the glyph's cluster starts.
    start: usize,
    key: CacheKey,
    /// Offset from the cell's top-left to the glyph origin, physical px.
    x: i32,
    y: i32,
}

/// Runs cached before the cache is reset (bounded memory for long sessions).
const MAX_CACHED_RUNS: usize = 8192;

/// What to draw in one frame of one pane.
pub struct Frame<'a, T> {
    pub term: &'a Term<T>,
    pub theme: &'a Theme,
    /// The pane has keyboard focus (solid, blinking cursor).
    pub focused: bool,
    /// Blink phase of the focused pane's cursor.
    pub cursor_visible: bool,
    /// Extra highlighted ranges (find matches, hovered link).
    pub highlights: &'a [Highlight],
    /// Cells with the default background are left see-through (the pane's
    /// background colour/image, drawn by the UI, shows instead).
    pub transparent: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HighlightKind {
    /// A find match (background tint).
    Match,
    /// A link under the pointer (underline).
    Link,
}

/// An inclusive range of grid points (scrollback coordinates).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Highlight {
    pub start: Point,
    pub end: Point,
    pub kind: HighlightKind,
}

/// Font settings a renderer is built for.
#[derive(Clone, Debug, PartialEq)]
pub struct FontSpec {
    /// Preferred family; a fallback monospace font is used if missing.
    pub face: String,
    /// Physical pixels (points × scale factor × 96/72).
    pub size: f32,
    /// CSS-style weight, 100–950.
    pub weight: u16,
    /// Line height as a multiple of the size.
    pub cell_height: f32,
    /// Join character sequences into ligatures (e.g. `->`, `!=`) when the
    /// font has them.
    pub ligatures: bool,
}

impl Renderer {
    /// `font_size` is in physical pixels (points × scale factor × 96/72).
    /// `fonts` comes from [`load_fonts`], which is slow enough (hundreds of
    /// system fonts on Windows) that callers should run it off the UI thread.
    /// `face` is the preferred font family; if it isn't installed, a
    /// fallback monospace font is used.
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat, font: &FontSpec, fonts: SharedFonts) -> Renderer {
        let family = pick_family(&fonts.borrow(), &font.face);
        log::info!("font {:?} → using {family:?}", font.face);
        let font_size = font.size;

        let atlas = Atlas::new(device);
        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/cells.wgsl"));

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wtpp globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("wtpp atlas sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("wtpp cells"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
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
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wtpp cells"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&atlas.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("wtpp cells"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("wtpp cells"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x4, 4 => Uint32
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    // The shader writes premultiplied colour, so a see-through
                    // frame keeps correct alpha; over an opaque clear this is
                    // the same as straight alpha blending.
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let resolve_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/resolve.wgsl"));
        let resolve_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("wtpp resolve"),
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
        let resolve_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("wtpp resolve"),
            bind_group_layouts: &[Some(&resolve_layout)],
            immediate_size: 0,
        });
        let resolve_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("wtpp resolve"),
            layout: Some(&resolve_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &resolve_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &resolve_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: target_format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let instance_capacity = 4096;
        let instances = create_instance_buffer(device, instance_capacity);

        let mut renderer = Renderer {
            pipeline,
            bind_group,
            globals,
            instances,
            instance_capacity,
            atlas,
            cell: CellMetrics { width: 1.0, height: 1.0 },
            swash: SwashCache::new(),
            family,
            font_size,
            weight: font.weight,
            cell_height: font.cell_height.clamp(0.8, 3.0),
            glyphs: HashMap::new(),
            runs: HashMap::new(),
            ligatures: font.ligatures,
            atlas_full: false,
            fonts,
            resolve_pipeline,
            resolve_layout,
            scratch: HashMap::new(),
        };
        renderer.cell = renderer.measure_cell();
        renderer
    }

    pub fn cell_metrics(&self) -> CellMetrics {
        self.cell
    }

    pub fn font_size(&self) -> f32 {
        self.font_size
    }

    /// Change the font size (physical pixels); drops all cached glyphs.
    pub fn set_font_size(&mut self, font_size: f32) {
        if font_size == self.font_size {
            return;
        }
        self.font_size = font_size;
        self.glyphs.clear();
        self.runs.clear();
        self.atlas.clear();
        self.cell = self.measure_cell();
    }

    fn line_height(&self) -> f32 {
        (self.font_size * self.cell_height).round()
    }

    fn shape(&mut self, text: &str, bold: bool, italic: bool) -> Buffer {
        let metrics = Metrics::new(self.font_size, self.line_height());
        let attrs = Attrs::new()
            .family(Family::Name(&self.family))
            .weight(if bold { Weight(self.weight.max(400).saturating_add(300).min(900)) } else { Weight(self.weight) })
            .style(if italic { Style::Italic } else { Style::Normal });
        let mut fonts = self.fonts.borrow_mut();
        let mut buffer = Buffer::new(&mut fonts, metrics);
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut fonts, false);
        buffer
    }

    fn measure_cell(&mut self) -> CellMetrics {
        let buffer = self.shape("M", false, false);
        let width = buffer
            .layout_runs()
            .next()
            .and_then(|run| run.glyphs.first().map(|g| g.w))
            .unwrap_or(self.font_size * 0.6);
        CellMetrics { width: width.round().max(1.0), height: self.line_height() }
    }

    /// Shape a run of text (cached).
    fn shape_run(&mut self, text: &str, bold: bool, italic: bool) -> Rc<[ShapedGlyph]> {
        let key = (text.to_string(), bold, italic);
        if let Some(run) = self.runs.get(&key) {
            return run.clone();
        }
        if self.runs.len() >= MAX_CACHED_RUNS {
            self.runs.clear();
        }
        let buffer = self.shape(text, bold, italic);
        let shaped: Rc<[ShapedGlyph]> = buffer
            .layout_runs()
            .flat_map(|run| {
                run.glyphs.iter().map(move |g| {
                    // Position relative to where the glyph's cell starts:
                    // the grid, not the shaper's advances, decides x.
                    let physical = g.physical((-g.x, 0.0), 1.0);
                    ShapedGlyph { start: g.start, key: physical.cache_key, x: physical.x, y: run.line_y as i32 + physical.y }
                })
            })
            .collect();
        self.runs.insert(key, shaped.clone());
        shaped
    }

    fn glyph(&mut self, queue: &wgpu::Queue, key: CacheKey) -> Option<Glyph> {
        if let Some(cached) = self.glyphs.get(&key) {
            return *cached;
        }
        let glyph = self.rasterize(queue, key);
        self.glyphs.insert(key, glyph);
        glyph
    }

    fn rasterize(&mut self, queue: &wgpu::Queue, key: CacheKey) -> Option<Glyph> {
        let image = self.swash.get_image_uncached(&mut self.fonts.borrow_mut(), key)?;
        let (w, h) = (image.placement.width, image.placement.height);
        let rgba: Vec<u8> = match image.content {
            SwashContent::Mask => image.data.iter().flat_map(|&a| [255, 255, 255, a]).collect(),
            SwashContent::SubpixelMask => image
                .data
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [255, 255, 255, ((p[0] as u16 + p[1] as u16 + p[2] as u16) / 3) as u8])
                .collect(),
            SwashContent::Color => image.data.clone(),
        };
        let Some(atlas_pos) = self.atlas.insert(queue, w, h, &rgba) else {
            self.atlas_full = true;
            return None;
        };
        Some(Glyph {
            atlas_pos,
            size: [w, h],
            offset: [image.placement.left, -image.placement.top],
            color: image.content == SwashContent::Color,
        })
    }

    /// Glyph quads for one row of cells, shaped in runs of the same style so
    /// the font can form ligatures. Each glyph is drawn at the cell its
    /// cluster starts in.
    fn row_glyphs(&mut self, queue: &wgpu::Queue, y: f32, cells: &[TextCell], out: &mut Vec<Instance>) {
        let cw = self.cell.width;
        let mut start = 0;
        while start < cells.len() {
            let style = (cells[start].bold, cells[start].italic);
            let mut end = start + 1;
            // Without ligatures every cell is its own run; the cursor cell
            // always is, so a ligature never hides it.
            if self.ligatures && !cells[start].breaks_run {
                while end < cells.len()
                    && (cells[end].bold, cells[end].italic) == style
                    && cells[end].col == cells[end - 1].col + cells[end - 1].width
                    && !cells[end].breaks_run
                {
                    end += 1;
                }
            }
            let run = &cells[start..end];
            // Byte offset → cell, for placing each glyph.
            let mut text = String::new();
            let mut starts = Vec::with_capacity(run.len());
            for cell in run {
                starts.push(text.len());
                text.push_str(&cell.text);
            }
            if !text.trim().is_empty() {
                let shaped = self.shape_run(&text, style.0, style.1);
                for g in shaped.iter() {
                    let i = starts.partition_point(|&s| s <= g.start).saturating_sub(1);
                    let cell = &run[i];
                    if let Some(glyph) = self.glyph(queue, g.key) {
                        out.push(Instance {
                            pos: [
                                cell.col as f32 * cw + (g.x + glyph.offset[0]) as f32,
                                y + (g.y + glyph.offset[1]) as f32,
                            ],
                            size: [glyph.size[0] as f32, glyph.size[1] as f32],
                            uv_pos: [glyph.atlas_pos[0] as f32, glyph.atlas_pos[1] as f32],
                            color: theme::to_f32(cell.fg, 1.0),
                            kind: if glyph.color { KIND_COLOR } else { KIND_MASK },
                        });
                    }
                }
            }
            start = end;
        }
    }

    /// Draw the terminal's current viewport into `target`, which is
    /// `width`×`height` physical pixels.
    pub fn render<T: EventListener>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        width: u32,
        height: u32,
        frame: &Frame<'_, T>,
    ) {
        let theme = frame.theme;
        let instances = self.build_instances(queue, frame);

        if instances.len() > self.instance_capacity {
            self.instance_capacity = instances.len().next_power_of_two();
            self.instances = create_instance_buffer(device, self.instance_capacity);
        }
        queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&instances));
        queue.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&Globals {
                screen: [width as f32, height as f32],
                atlas: [ATLAS_SIZE as f32; 2],
            }),
        );

        let bg = theme.background;
        let mut encoder = device.create_command_encoder(&Default::default());
        if frame.transparent {
            // Draw premultiplied into a scratch texture, then write it out
            // as straight alpha (how Slint reads the pane's texture).
            if !self.scratch.contains_key(&(width, height)) && self.scratch.len() >= MAX_SCRATCH {
                self.scratch.clear();
            }
            let layout = &self.resolve_layout;
            let (scratch, resolve_group) = self.scratch.entry((width, height)).or_insert_with(|| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("wtpp scratch"),
                    size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let view = texture.create_view(&Default::default());
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("wtpp resolve"),
                    layout,
                    entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
                });
                (texture, group)
            });
            let scratch_view = scratch.create_view(&Default::default());
            let resolve_group = resolve_group.clone();
            self.draw_cells(&mut encoder, &scratch_view, wgpu::Color::TRANSPARENT, instances.len());
            let mut pass = begin_pass(&mut encoder, "wtpp resolve", target, wgpu::Color::TRANSPARENT);
            pass.set_pipeline(&self.resolve_pipeline);
            pass.set_bind_group(0, &resolve_group, &[]);
            pass.draw(0..3, 0..1);
        } else {
            let clear = wgpu::Color { r: bg.r as f64 / 255.0, g: bg.g as f64 / 255.0, b: bg.b as f64 / 255.0, a: 1.0 };
            self.draw_cells(&mut encoder, target, clear, instances.len());
        }
        queue.submit([encoder.finish()]);
    }

    fn draw_cells(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, clear: wgpu::Color, count: usize) {
        let mut pass = begin_pass(encoder, "wtpp cells", target, clear);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..4, 0..count as u32);
    }

    fn build_instances<T: EventListener>(&mut self, queue: &wgpu::Queue, frame: &Frame<'_, T>) -> Vec<Instance> {
        if self.atlas_full {
            // Glyphs missing from the last frame get re-rasterized now.
            self.atlas_full = false;
            self.atlas.clear();
            self.glyphs.clear();
        }
        let (theme, focused) = (frame.theme, frame.focused);
        // Unfocused panes show a steady hollow cursor.
        let cursor_visible = frame.cursor_visible || !focused;
        let content = frame.term.renderable_content();
        let CellMetrics { width: cw, height: ch } = self.cell;
        let offset = content.display_offset as i32;
        let cursor = content.cursor;
        let block_cursor = focused && cursor_visible && cursor.shape == CursorShape::Block;
        let selection = content.selection;
        let colors = content.colors;

        let mut backgrounds = Vec::new();
        let mut glyphs = Vec::new();
        let mut overlays = Vec::new();
        // Cells of the row being collected, shaped together when it ends.
        let mut row_cells: Vec<TextCell> = Vec::new();
        let mut text_row = i32::MIN;

        for indexed in content.display_iter {
            let cell = &indexed.cell;
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let row = indexed.point.line.0 + offset;
            let x = indexed.point.column.0 as f32 * cw;
            let y = row as f32 * ch;
            let cols = if cell.flags.contains(Flags::WIDE_CHAR) { 2.0 } else { 1.0 };

            let bold = cell.flags.contains(Flags::BOLD);
            let mut fg = theme.resolve(cell.fg, colors, bold && theme.bold_is_bright);
            let mut bg = theme.resolve(cell.bg, colors, false);
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.flags.contains(Flags::DIM) {
                fg = theme::dim(fg);
            }
            let is_block_cursor = block_cursor && indexed.point == cursor.point;
            if is_block_cursor {
                fg = theme.background;
                bg = theme.cursor;
            }

            let inverse = cell.flags.contains(Flags::INVERSE);
            if paints_background(cell.bg, inverse, is_block_cursor, bg, theme.background, frame.transparent) {
                backgrounds.push(Instance::solid(x, y, cw * cols, ch, theme::to_f32(bg, 1.0)));
            }
            if selection.is_some_and(|s| s.contains(indexed.point)) {
                backgrounds.push(Instance::solid(x, y, cw * cols, ch, theme::to_f32(theme.selection_background, 0.3)));
            }
            let mut link = false;
            for h in frame.highlights {
                if h.start <= indexed.point && indexed.point <= h.end {
                    match h.kind {
                        HighlightKind::Match => backgrounds.push(Instance::solid(
                            x,
                            y,
                            cw * cols,
                            ch,
                            theme::to_f32(MATCH_HIGHLIGHT, 0.35),
                        )),
                        HighlightKind::Link => link = true,
                    }
                }
            }

            if row != text_row {
                self.row_glyphs(queue, text_row as f32 * ch, &row_cells, &mut glyphs);
                row_cells.clear();
                text_row = row;
            }
            let mut text = String::new();
            if !cell.flags.contains(Flags::HIDDEN) {
                text.push(if cell.c == '\t' { ' ' } else { cell.c });
                // Combining marks belong to this cell's cluster.
                text.extend(cell.zerowidth().into_iter().flatten());
            } else {
                text.push(' ');
            }
            row_cells.push(TextCell {
                col: indexed.point.column.0,
                width: cols as usize,
                text,
                fg,
                bold: bold && theme.bold_font,
                italic: cell.flags.contains(Flags::ITALIC),
                breaks_run: indexed.point == cursor.point,
            });

            let thickness = (ch / 16.0).round().max(1.0);
            let fg4 = theme::to_f32(fg, 1.0);
            if link || cell.flags.intersects(Flags::ALL_UNDERLINES) {
                overlays.push(Instance::solid(x, y + ch - 2.0 * thickness, cw * cols, thickness, fg4));
            }
            if cell.flags.contains(Flags::STRIKEOUT) {
                overlays.push(Instance::solid(x, y + ch / 2.0, cw * cols, thickness, fg4));
            }
        }

        self.row_glyphs(queue, text_row as f32 * ch, &row_cells, &mut glyphs);

        // Non-block cursors draw on top of the text.
        let cursor_row = cursor.point.line.0 + offset;
        let (cx, cy) = (cursor.point.column.0 as f32 * cw, cursor_row as f32 * ch);
        let cursor_color = theme::to_f32(theme.cursor, 1.0);
        let shape = match (focused, cursor_visible) {
            (_, false) => CursorShape::Hidden,
            (true, true) => cursor.shape,
            (false, true) if cursor.shape == CursorShape::Hidden => CursorShape::Hidden,
            (false, true) => CursorShape::HollowBlock,
        };
        let t = (cw / 8.0).round().max(1.0);
        match shape {
            CursorShape::Beam => overlays.push(Instance::solid(cx, cy, t, ch, cursor_color)),
            CursorShape::Underline => overlays.push(Instance::solid(cx, cy + ch - 2.0 * t, cw, 2.0 * t, cursor_color)),
            CursorShape::HollowBlock => {
                overlays.push(Instance::solid(cx, cy, cw, t, cursor_color));
                overlays.push(Instance::solid(cx, cy + ch - t, cw, t, cursor_color));
                overlays.push(Instance::solid(cx, cy, t, ch, cursor_color));
                overlays.push(Instance::solid(cx + cw - t, cy, t, ch, cursor_color));
            }
            CursorShape::Block | CursorShape::Hidden => {}
        }

        backgrounds.extend(glyphs);
        backgrounds.extend(overlays);
        backgrounds
    }
}

fn begin_pass<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    label: &str,
    target: &wgpu::TextureView,
    clear: wgpu::Color,
) -> wgpu::RenderPass<'e> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("wtpp instances"),
        size: (capacity * std::mem::size_of::<Instance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Fonts shipped with wt++ (SIL Open Font License, see assets/fonts), so the
/// default looks the same everywhere, as Windows Terminal bundles them too.
const BUNDLED_FONTS: [&[u8]; 5] = [
    include_bytes!("../../../assets/fonts/CascadiaMono.ttf"),
    // Only for the Powerline symbols (prompt arrows) the others lack; see
    // `Fallback` below.
    include_bytes!("../../../assets/fonts/CascadiaMonoPL.ttf"),
    include_bytes!("../../../assets/fonts/CascadiaMonoItalic.ttf"),
    include_bytes!("../../../assets/fonts/CascadiaCode.ttf"),
    include_bytes!("../../../assets/fonts/CascadiaCodeItalic.ttf"),
];

/// Load the system font database, plus the bundled fonts.
pub fn load_fonts() -> FontSystem {
    let mut db = cosmic_text::fontdb::Database::new();
    db.load_system_fonts();
    for data in BUNDLED_FONTS {
        db.load_font_data(data.to_vec());
    }
    let locale = sys_locale().unwrap_or_else(|| "en-US".into());
    FontSystem::new_with_locale_and_db_and_fallback(locale, db, Fallback::new())
}

fn sys_locale() -> Option<String> {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|v| !v.is_empty()))
        .map(|l| l.split(['.', '@']).next().unwrap_or_default().replace('_', "-"))
}

/// The platform's fallback fonts, with the bundled Cascadia Mono PL first:
/// characters Cascadia Mono lacks (Powerline prompt symbols, in the private
/// use area) would otherwise come from whatever system font claims those
/// code points — on macOS, an Arabic one.
struct Fallback {
    platform: cosmic_text::PlatformFallback,
    common: &'static [&'static str],
}

impl Fallback {
    fn new() -> Fallback {
        use cosmic_text::Fallback as _;
        let platform = cosmic_text::PlatformFallback;
        let common: Vec<&'static str> =
            std::iter::once("Cascadia Mono PL").chain(platform.common_fallback().iter().copied()).collect();
        // Built once per font system (one per process).
        Fallback { platform, common: Box::leak(common.into_boxed_slice()) }
    }
}

impl cosmic_text::Fallback for Fallback {
    fn common_fallback(&self) -> &[&'static str] {
        self.common
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        self.platform.forbidden_fallback()
    }

    fn script_fallback(&self, script: unicode_script::Script, locale: &str) -> &[&'static str] {
        self.platform.script_fallback(script, locale)
    }
}

/// Names of the installed monospace font families, sorted.
pub fn monospace_families(fonts: &FontSystem) -> Vec<String> {
    let mut names: Vec<String> = fonts
        .db()
        .faces()
        .filter(|f| f.monospaced)
        .filter_map(|f| f.families.first().map(|(n, _)| n.clone()))
        .collect();
    names.sort_unstable_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

fn pick_family(fonts: &FontSystem, preferred: &str) -> String {
    let installed =
        |name: &str| fonts.db().faces().any(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)));
    std::iter::once(&preferred)
        .chain(FALLBACK_FONTS)
        .find(|name| installed(name))
        .map(|s| s.to_string())
        .unwrap_or_else(|| "monospace".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BG: Rgb = Rgb { r: 12, g: 12, b: 12 };
    const RED: Rgb = Rgb { r: 200, g: 0, b: 0 };

    #[test]
    fn default_background_is_left_out() {
        let default = Color::Named(NamedColor::Background);
        assert!(!paints_background(default, false, false, BG, BG, true));
        assert!(!paints_background(default, false, false, BG, BG, false));
    }

    #[test]
    fn explicit_background_covers_the_image() {
        // The scheme's colour, chosen explicitly, still paints when see-through.
        let same = Color::Spec(BG);
        assert!(paints_background(same, false, false, BG, BG, true));
        assert!(!paints_background(same, false, false, BG, BG, false));
        assert!(paints_background(Color::Spec(RED), false, false, RED, BG, true));
        assert!(paints_background(Color::Named(NamedColor::Red), false, false, RED, BG, false));
    }

    #[test]
    fn shaders_are_valid() {
        use wgpu::naga;
        for src in [include_str!("shaders/cells.wgsl"), include_str!("shaders/resolve.wgsl")] {
            let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::default())
                .validate(&module)
                .unwrap();
        }
    }

    #[test]
    fn inverse_and_cursor_cells_paint() {
        let default = Color::Named(NamedColor::Background);
        assert!(paints_background(default, true, false, Rgb { r: 200, g: 200, b: 200 }, BG, true));
        assert!(paints_background(default, false, true, RED, BG, true));
        // Opaque mode keeps the colour comparison.
        assert!(paints_background(default, false, true, RED, BG, false));
        assert!(!paints_background(default, true, false, BG, BG, false));
    }
}
