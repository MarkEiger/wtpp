// Copies a premultiplied frame out as straight alpha, the way Slint reads
// a pane's texture.

@group(0) @binding(0) var src: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    // One triangle covering the whole target.
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = textureLoad(src, vec2<i32>(pos.xy), 0);
    if p.a <= 0.0 {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(p.rgb / p.a, p.a);
}
