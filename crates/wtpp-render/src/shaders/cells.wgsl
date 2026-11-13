// Every cell background, decoration and glyph is one instanced quad.

struct Globals {
    screen: vec2<f32>,
    atlas: vec2<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas_tex: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct Instance {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv_pos: vec2<f32>,
    @location(3) color: vec4<f32>,
    // 0 = solid rect, 1 = mask glyph (tinted by color), 2 = color glyph (emoji)
    @location(4) kind: u32,
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) kind: u32,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Instance) -> VsOut {
    // Triangle strip corners: (0,0) (1,0) (0,1) (1,1)
    let corner = vec2<f32>(f32(vi & 1u), f32(vi >> 1u));
    let px = inst.pos + corner * inst.size;
    var out: VsOut;
    out.clip = vec4<f32>(px.x / globals.screen.x * 2.0 - 1.0, 1.0 - px.y / globals.screen.y * 2.0, 0.0, 1.0);
    out.uv = (inst.uv_pos + corner * inst.size) / globals.atlas;
    out.color = inst.color;
    out.kind = inst.kind;
    return out;
}

// Output is premultiplied (the pipeline blends with One, OneMinusSrcAlpha),
// so a see-through frame ends up with correct colour and alpha.
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if in.kind == 0u {
        return vec4<f32>(in.color.rgb * in.color.a, in.color.a);
    }
    let texel = textureSampleLevel(atlas_tex, atlas_sampler, in.uv, 0.0);
    if in.kind == 1u {
        let a = in.color.a * texel.a;
        return vec4<f32>(in.color.rgb * a, a);
    }
    // Colour glyph bitmaps are read as straight alpha.
    return vec4<f32>(texel.rgb * texel.a, texel.a);
}
