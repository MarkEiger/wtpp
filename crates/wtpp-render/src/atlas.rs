//! Glyph texture atlas with simple shelf packing.

pub const ATLAS_SIZE: u32 = 2048;

pub struct Atlas {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    shelf_x: u32,
    shelf_y: u32,
    shelf_height: u32,
}

impl Atlas {
    pub fn new(device: &wgpu::Device) -> Atlas {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("wtpp glyph atlas"),
            size: wgpu::Extent3d { width: ATLAS_SIZE, height: ATLAS_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Atlas { texture, view, shelf_x: 0, shelf_y: 0, shelf_height: 0 }
    }

    /// Forget all packed glyphs. Old texels stay but are never sampled.
    pub fn clear(&mut self) {
        self.shelf_x = 0;
        self.shelf_y = 0;
        self.shelf_height = 0;
    }

    /// Upload an RGBA image and return its top-left position in the atlas,
    /// or `None` if the atlas is full.
    pub fn insert(&mut self, queue: &wgpu::Queue, width: u32, height: u32, rgba: &[u8]) -> Option<[u32; 2]> {
        const PAD: u32 = 1;
        if self.shelf_x + width + PAD > ATLAS_SIZE {
            self.shelf_x = 0;
            self.shelf_y += self.shelf_height + PAD;
            self.shelf_height = 0;
        }
        if self.shelf_y + height > ATLAS_SIZE || width > ATLAS_SIZE {
            // TODO(render): evict and repack instead of giving up once
            // scrollback with lots of CJK/emoji makes this reachable.
            return None;
        }
        let pos = [self.shelf_x, self.shelf_y];
        if width > 0 && height > 0 {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: pos[0], y: pos[1], z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                rgba,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: None },
                wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            );
        }
        self.shelf_x += width + PAD;
        self.shelf_height = self.shelf_height.max(height);
        Some(pos)
    }
}
