struct Vertex {
    @location(0) position: vec2f,
    @location(1) light: vec4f,
    @location(2) uv: vec2f,
    @location(3) dark: vec4f,
    @location(4) clip: vec2f,
};
struct Fragment {
    @builtin(position) position: vec4f,
    @location(0) light: vec4f,
    @location(1) uv: vec2f,
    @location(2) dark: vec4f,
    @location(3) clip: vec2f,
};
@group(0) @binding(0) var atlas: texture_2d<f32>;
@group(0) @binding(1) var filtering: sampler;
@vertex fn vs(v: Vertex) -> Fragment {
    return Fragment(vec4f(v.position, 0., 1.), v.light, v.uv, v.dark, v.clip);
}
@fragment fn fs(v: Fragment) -> @location(0) vec4f {
    let texel = textureSample(atlas, filtering, v.uv);
    if any(v.clip < vec2f(0.)) || any(v.clip >= vec2f(1.)) { discard; }
    return vec4f((texel.a - texel.rgb)*v.dark.rgb + texel.rgb*v.light.rgb, texel.a*v.light.a);
}
@vertex fn resolve_vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let positions = array<vec2f,3>(vec2f(-1.,-1.),vec2f(3.,-1.),vec2f(-1.,3.));
    return vec4f(positions[i],0.,1.);
}
@fragment fn resolve_fs(@builtin(position) p: vec4f) -> @location(0) vec4f {
    let color = textureLoad(atlas, vec2i(p.xy), 0);
    let alpha = clamp(color.a,0.,1.);
    if alpha < 0.5/255. { return vec4f(0.); }
    return vec4f(color.rgb/alpha, alpha);
}

// Match the desktop's straight-RGBA8 -> PMA packing, including quantization.
@fragment fn resolve_native_fs(@builtin(position) p: vec4f) -> @location(0) vec4f {
    let color = textureLoad(atlas, vec2i(p.xy), 0);
    let alpha = clamp(color.a, 0., 1.);
    if alpha < 0.5/255. { return vec4f(0.); }
    let a8 = round(alpha * 255.);
    let rgb8 = round(clamp(color.rgb / alpha, vec3f(0.), vec3f(1.)) * 255.);
    let pma = floor((rgb8 * a8 + 127.) / 255.) / 255.;
    return vec4f(pma.bgr, a8 / 255.);
}
