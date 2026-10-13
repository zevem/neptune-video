// What every pass of the compositor is given. One pass draws one triangle
// that covers its target and reads its texels at the pixel it writes.
//
// The intermediate targets are `Rgba8UnormSrgb`: a shader reads and writes
// linear light, premultiplied by alpha, and the GPU does the encoding. Only
// `encode.wgsl` writes the gamma-encoded bytes the interface is drawn with.
//
// The fields mean the same in every pass, and `p` is the pass's own: its
// shader says what each vector holds. Everything is a `vec4`, so the layout
// here and in `pass.rs` cannot drift apart over padding.
struct Pass {
    // x: the operation of an effect pass. y: its variant, or the blend mode.
    // z: how many records `p` holds. w: unused.
    op: vec4<u32>,
    // xy: the target in pixels. zw: one pixel as a fraction of it.
    size: vec4<f32>,
    // xy: noise seed, each in 0..1 and the same for one clip on one canvas
    // frame. z: how much of an effect is mixed in. w: the layer's opacity.
    seed: vec4<f32>,
    p: array<vec4<f32>, 28>,
}

@group(0) @binding(0) var<uniform> u: Pass;
@group(0) @binding(1) var first: texture_2d<f32>;
@group(0) @binding(2) var second: texture_2d<f32>;
@group(0) @binding(3) var smooth_clamp: sampler;

@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

// `at` about `centre`, turned back by the angle whose cosine and sine are
// `turn`: a target pixel in the axes of a rotated box.
fn into_box(at: vec2<f32>, centre: vec2<f32>, turn: vec2<f32>) -> vec2<f32> {
    let d = at - centre;
    return vec2<f32>(d.x * turn.x + d.y * turn.y, -d.x * turn.y + d.y * turn.x);
}

fn pcg(n: u32) -> u32 {
    var h = n * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (h >> 22u) ^ h;
}

// The two fractions of a seed as the integer they were cut from.
fn salt_of(seed: vec2<f32>) -> u32 {
    return u32(seed.x * 16777216.0) ^ (u32(seed.y * 16777216.0) << 8u);
}

// A number in 0..1 for a cell: the same for the same cell and salt on every
// device, since nothing here is a float until the end.
fn noise(cell: vec2<u32>, salt: u32) -> f32 {
    return f32(pcg(cell.x + pcg(cell.y + pcg(salt))) >> 8u) / 16777216.0;
}
