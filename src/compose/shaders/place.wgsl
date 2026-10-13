// The layer pass: one decoded picture, cropped, flipped, scaled and turned
// into its place on a transparent canvas, with the alpha it came with.
//
// first: the picture, straight alpha, decoded to linear light by its format.
// p[0]: the box's centre (xy) and half size (zw), in target pixels.
// p[1]: cosine and sine of its angle, clockwise.
// p[2]: the picture's uv at the box's left top (xy) and right bottom (zw).
// p[3]: how a stabilized picture is read, as the rows of a matrix (xy, zw)
//       about the picture's centre: the identity for any other.
// p[4]: and what that reading is moved by (xy), in fractions of the picture.
//
// What the box shows of the picture at `uv` is read at
// `0.5 + p[3] * (uv - 0.5) + p[4].xy`: the camera's shake undone and the
// crop that hides the borders it would bring in, in one step, on the
// picture's own texels. A reading past the picture's edge holds the edge.

fn texel(at: vec2<i32>, last: vec2<i32>) -> vec4<f32> {
    let c = textureLoad(first, clamp(at, vec2<i32>(0), last), 0);
    return vec4<f32>(c.rgb * c.a, c.a);
}

@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let half = max(u.p[0].zw, vec2<f32>(0.0001));
    let local = into_box(at.xy, u.p[0].xy, u.p[1].xy);
    // A pixel the edge passes through is covered by the part inside it.
    let edge = clamp(half - abs(local) + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
    let cover = edge.x * edge.y;
    if cover <= 0.0 {
        return vec4<f32>(0.0);
    }
    let q = clamp(local / (2.0 * half) + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
    let shown = mix(u.p[2].xy, u.p[2].zw, q) - vec2<f32>(0.5);
    let uv = vec2<f32>(0.5) + u.p[4].xy
        + vec2<f32>(dot(u.p[3].xy, shown), dot(u.p[3].zw, shown));
    // Filtered by hand over premultiplied texels, so a transparent texel
    // lends no colour to the edge of an opaque one.
    let size = vec2<f32>(textureDimensions(first));
    let pos = uv * size - 0.5;
    let base = floor(pos);
    let f = pos - base;
    let i = vec2<i32>(base);
    let last = vec2<i32>(size) - 1;
    let top = mix(texel(i, last), texel(i + vec2<i32>(1, 0), last), f.x);
    let bottom = mix(texel(i + vec2<i32>(0, 1), last), texel(i + vec2<i32>(1, 1), last), f.x);
    return mix(top, bottom, f.y) * cover;
}
