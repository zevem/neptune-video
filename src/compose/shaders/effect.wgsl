// The effect pass: one step of a layer's chain, from one canvas-sized target
// to another. `u.op.x` picks the operation and `u.seed.z` mixes its result
// over what came in.
//
// Adding an effect is one function here, one `case` below and one arm of
// `Chain::effect` in `pass.rs`. An effect's parameters arrive in `p`, four to
// a vector from `p[0]`, in the order of its table in `editor_core::look`;
// what `pass.rs` works out for it is in `p[4]` on.
//
// first: the layer so far, or what the pass before made of it.
// second: what the operation asks for, else blank. With `u.op.w` set it is
// the layer as the effect found it, which is then what the result is mixed
// over.
//
// Every texel is linear light multiplied by its alpha. An operation on
// colour works on the colour itself, and one that says "gamma-encoded" on
// the sRGB encoding of it, where equal steps look equal.

const OP_COPY: u32 = 0u;
const OP_MASK: u32 = 1u;
const OP_GRADE: u32 = 2u;
const OP_HSL_BAND: u32 = 3u;
const OP_CURVE: u32 = 4u;
const OP_CHROMA_KEY: u32 = 5u;
const OP_DIFFERENCE_MATTE: u32 = 6u;
const OP_GRAIN: u32 = 7u;
const OP_VIGNETTE: u32 = 8u;
const OP_SHAKE: u32 = 9u;
const OP_DISTORT: u32 = 10u;
const OP_GLITCH: u32 = 11u;
const OP_STREAK: u32 = 12u;
const OP_SHRINK: u32 = 13u;
const OP_GAUSS: u32 = 14u;
const OP_SOFT: u32 = 15u;
const OP_BRIGHT: u32 = 16u;
const OP_LUT: u32 = 17u;

// The variants of `OP_SOFT`: what is made of the softened layer.
const SOFT_BLUR: u32 = 0u;
const SOFT_GLOW: u32 = 1u;
const SOFT_SHARPEN: u32 = 2u;
const SOFT_SHADOW: u32 = 3u;

// Rec. 709: how bright a linear colour is.
const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);
// The grey that contrast turns about.
const MID_GREY: f32 = 0.18;
const TAU: f32 = 6.283185307;

fn straight(c: vec4<f32>) -> vec3<f32> {
    if c.a <= 0.0 {
        return vec3<f32>(0.0);
    }
    return clamp(c.rgb / c.a, vec3<f32>(0.0), vec3<f32>(1.0));
}

// `rgb` in the place of the colour of `c`, with the alpha `c` has.
fn coloured(c: vec4<f32>, rgb: vec3<f32>) -> vec4<f32> {
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * c.a, c.a);
}

fn to_gamma(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((max(c, vec3<f32>(0.0)) + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

// The layer at a place between its pixels, and nothing past the canvas.
fn pick(at: vec2<f32>) -> vec4<f32> {
    let uv = at * u.size.zw;
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) {
        return vec4<f32>(0.0);
    }
    return textureSampleLevel(first, smooth_clamp, uv, 0.0);
}

// How much of a pixel the layer's masks leave.
//
// p[0]: the layer box's centre (xy) and half size (zw), in target pixels.
// p[1]: cosine and sine of its angle.
// Then three vectors a mask, `u.op.z` of them:
//   centre (xy) and size (zw) as fractions of the box, origin left top;
//   cosine and sine of its own angle, feather, roundness;
//   shape (0 rectangle, 1 ellipse), op (0 add, 1 subtract, 2 intersect),
//   invert.
// Feather is the width of the soft band, centred on the edge, as a fraction
// of the box's shorter side; an unfeathered edge is one pixel wide.
fn mask_cover(at: vec2<f32>) -> f32 {
    let local = into_box(at, u.p[0].xy, u.p[1].xy);
    let box = 2.0 * u.p[0].zw;
    let unit = min(box.x, box.y);
    var cover = 0.0;
    for (var k = 0u; k < min(u.op.z, 8u); k++) {
        let place = u.p[2u + 3u * k];
        let shape = u.p[3u + 3u * k];
        let kind = u.p[4u + 3u * k];
        let half = max(place.zw * 0.5 * box, vec2<f32>(0.0001));
        let q = into_box(local, (place.xy - 0.5) * box, shape.xy);
        var away: f32;
        if kind.x < 0.5 {
            let radius = shape.w * min(half.x, half.y);
            let e = abs(q) - half + radius;
            away = length(max(e, vec2<f32>(0.0))) + min(max(e.x, e.y), 0.0) - radius;
        } else {
            let near = length(q / half);
            let slope = length(q / (half * half));
            if slope < 0.000001 {
                away = -min(half.x, half.y);
            } else {
                away = near * (near - 1.0) / slope;
            }
        }
        let band = max(shape.z * unit, 1.0);
        var inside = 1.0 - smoothstep(-0.5 * band, 0.5 * band, away);
        if kind.z > 0.5 {
            inside = 1.0 - inside;
        }
        // The first mask adds to nothing, or takes from everything.
        if k == 0u && kind.y > 0.5 {
            cover = 1.0;
        }
        if kind.y < 0.5 {
            cover = max(cover, inside);
        } else if kind.y < 1.5 {
            cover *= 1.0 - inside;
        } else {
            cover *= inside;
        }
    }
    return cover;
}

// Grade, on linear light, in this order: exposure in stops; white balance as
// gains that keep white as bright as it was; contrast as a power about mid
// grey; lift, gain and gamma; saturation about the colour's own brightness.
//
// p[0]: exposure, contrast, saturation, temperature. p[1]: tint, lift,
// gamma, gain. Temperature above nought is warmer, tint above nought is
// towards magenta, and gamma above one lifts the middle: the result is
// raised to one over it.
fn grade(c: vec3<f32>) -> vec3<f32> {
    let a = u.p[0];
    let b = u.p[1];
    var x = c * exp2(a.x);
    let balance = vec3<f32>(exp2(0.5 * a.w), exp2(-0.5 * b.x), exp2(-0.5 * a.w));
    x *= balance / dot(balance, LUMA);
    x = MID_GREY * pow(max(x, vec3<f32>(0.0)) / MID_GREY, vec3<f32>(exp2(a.y)));
    x = b.w * (x + b.y * (1.0 - x));
    x = pow(max(x, vec3<f32>(0.0)), vec3<f32>(1.0 / max(b.z, 0.01)));
    return mix(vec3<f32>(dot(x, LUMA)), x, a.z);
}

// Hue as a fraction of a turn, saturation and lightness.
fn to_hsl(c: vec3<f32>) -> vec3<f32> {
    let high = max(c.r, max(c.g, c.b));
    let low = min(c.r, min(c.g, c.b));
    let light = 0.5 * (high + low);
    let d = high - low;
    if d < 0.00001 {
        return vec3<f32>(0.0, 0.0, light);
    }
    var hue: f32;
    if high == c.r {
        hue = (c.g - c.b) / d;
    } else if high == c.g {
        hue = 2.0 + (c.b - c.r) / d;
    } else {
        hue = 4.0 + (c.r - c.g) / d;
    }
    let chroma = min(d / max(1.0 - abs(2.0 * light - 1.0), 0.00001), 1.0);
    return vec3<f32>(fract(hue / 6.0), chroma, light);
}

fn from_hsl(hsl: vec3<f32>) -> vec3<f32> {
    let k = fract(vec3<f32>(0.0, 8.0, 4.0) / 12.0 + hsl.x) * 12.0;
    let a = hsl.y * min(hsl.z, 1.0 - hsl.z);
    return hsl.z - a * max(vec3<f32>(-1.0), min(min(k - 3.0, 9.0 - k), vec3<f32>(1.0)));
}

// One band of hues shifted, on the gamma-encoded colour, where hue is what
// it looks to be.
//
// p[0]: the band's centre and whole width in degrees, softness, hue shift
// in degrees. p[1]: saturation shift, lightness shift.
// Softness is the part of each half of the band the shift fades out over;
// a colour with no saturation has no hue and is left alone.
fn hsl_band(c: vec3<f32>) -> vec3<f32> {
    var hsl = to_hsl(to_gamma(c));
    let away = abs(fract(hsl.x - u.p[0].x / 360.0 + 0.5) - 0.5) * 360.0;
    let half = 0.5 * u.p[0].y;
    let inner = half * (1.0 - u.p[0].z);
    var weight = 1.0 - smoothstep(inner, max(half, inner + 0.001), away);
    weight *= smoothstep(0.0, 0.1, hsl.y);
    hsl.x = fract(hsl.x + weight * u.p[0].w / 360.0 + 1.0);
    hsl.y = clamp(hsl.y * (1.0 + weight * u.p[1].x), 0.0, 1.0);
    let lift = weight * u.p[1].y;
    if lift > 0.0 {
        hsl.z += (1.0 - hsl.z) * lift;
    } else {
        hsl.z *= 1.0 + lift;
    }
    return to_linear(from_hsl(hsl));
}

// A curve through `u.op.z` points, each `p[k]`: x, y and the slope there.
// The slopes are worked out in `pass.rs` so that the curve never turns back
// between two points. Before the first point and after the last it is flat.
fn curve(v: f32) -> f32 {
    let n = min(u.op.z, 16u);
    if n < 2u {
        return v;
    }
    if v <= u.p[0].x {
        return u.p[0].y;
    }
    if v >= u.p[n - 1u].x {
        return u.p[n - 1u].y;
    }
    var k = 0u;
    for (var i = 1u; i + 1u < n; i++) {
        if v >= u.p[i].x {
            k = i;
        }
    }
    let lo = u.p[k];
    let hi = u.p[k + 1u];
    let h = max(hi.x - lo.x, 0.000001);
    let t = (v - lo.x) / h;
    let t2 = t * t;
    let t3 = t2 * t;
    return (2.0 * t3 - 3.0 * t2 + 1.0) * lo.y + (t3 - 2.0 * t2 + t) * h * lo.z
        + (3.0 * t2 - 2.0 * t3) * hi.y + (t3 - t2) * h * hi.z;
}

// The curve on the gamma-encoded channels `u.op.y` names: 1 red, 2 green,
// 4 blue.
fn curves(c: vec3<f32>) -> vec3<f32> {
    var v = to_gamma(c);
    if (u.op.y & 1u) != 0u {
        v.r = curve(v.r);
    }
    if (u.op.y & 2u) != 0u {
        v.g = curve(v.g);
    }
    if (u.op.y & 4u) != 0u {
        v.b = curve(v.b);
    }
    return to_linear(v);
}

// Blue and red difference of a gamma-encoded colour (BT.601): what is left
// of it without its brightness.
fn chroma(c: vec3<f32>) -> vec2<f32> {
    return vec2<f32>(
        dot(c, vec3<f32>(-0.168736, -0.331264, 0.5)),
        dot(c, vec3<f32>(0.5, -0.418688, -0.081312)),
    );
}

// A key colour made clear. A pixel is kept by how far its chroma is from the
// key's: nothing within the tolerance, all of it a softness further.
//
// p[0]: the key, gamma-encoded (xyz), tolerance. p[1]: softness, spill.
// p[4]: one in the channel the key is strongest in.
// Spill takes that channel down towards the greater of the other two, on
// what is kept.
fn chroma_key(c: vec4<f32>) -> vec4<f32> {
    var rgb = to_gamma(straight(c));
    let away = distance(chroma(rgb), chroma(u.p[0].xyz));
    let keep = smoothstep(u.p[0].w, u.p[0].w + max(u.p[1].x, 0.0001), away);
    let own = u.p[4].xyz;
    let others = rgb * (1.0 - own);
    let over = dot(rgb, own) - max(others.r, max(others.g, others.b));
    rgb -= own * max(over, 0.0) * u.p[1].y;
    let a = c.a * keep;
    return vec4<f32>(to_linear(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0))) * a, a);
}

// What differs from a clean plate is kept: the plate lies over the layer's
// box as the layer's own picture does.
//
// second: the plate, straight alpha. p[0]: tolerance, softness.
// p[4]: the box's centre and half size. p[5]: cosine and sine of its angle.
// p[6]: the plate's uv at the box's left top (xy) and right bottom (zw).
fn difference_matte(at: vec2<f32>, c: vec4<f32>) -> vec4<f32> {
    let half = max(u.p[4].zw, vec2<f32>(0.0001));
    let local = into_box(at, u.p[4].xy, u.p[5].xy);
    let q = clamp(local / (2.0 * half) + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
    let plate = textureSampleLevel(second, smooth_clamp, mix(u.p[6].xy, u.p[6].zw, q), 0.0);
    let away = distance(to_gamma(straight(c)), to_gamma(plate.rgb));
    return c * smoothstep(u.p[0].x, u.p[0].x + max(u.p[0].y, 0.0001), away);
}

// Grain: one number a cell of the canvas, added to the gamma-encoded colour
// of all three channels alike. At an amount of one it reaches a quarter of
// the way from black to white either way.
//
// p[0]: amount, size. p[4].x: a cell's side in pixels.
fn grain(at: vec2<f32>, c: vec3<f32>) -> vec3<f32> {
    let cell = vec2<u32>(at / max(u.p[4].x, 1.0));
    let n = noise(cell, salt_of(u.seed.xy)) - 0.5;
    return to_linear(clamp(to_gamma(c) + n * 0.5 * u.p[0].x, vec3<f32>(0.0), vec3<f32>(1.0)));
}

// How much light a vignette leaves at a pixel. The distance from the box's
// centre is one at the middle of an edge; the darkening begins at the radius
// and is whole a softness further out.
//
// p[0]: amount, radius, softness. p[4], p[5]: the layer's box.
fn vignette(at: vec2<f32>) -> f32 {
    let local = into_box(at, u.p[4].xy, u.p[5].xy);
    let away = length(local / max(u.p[4].zw, vec2<f32>(0.0001)));
    let dark = smoothstep(u.p[0].y, u.p[0].y + max(u.p[0].z, 0.0001), away);
    return 1.0 - u.p[0].x * dark;
}

// The layer moved and turned about its box's centre by what `pass.rs` drew
// for this frame. What it moves away from is left clear.
//
// p[4].xy: the box's centre. p[6]: the offset in pixels (xy), cosine and
// sine of the turn.
fn shake(at: vec2<f32>) -> vec4<f32> {
    let centre = u.p[4].xy;
    return pick(centre + into_box(at - u.p[6].xy, centre, u.p[6].zw));
}

// A wave across the layer: each point is moved along the wave's crests by a
// sine of how far along the wave's direction it is.
//
// p[0]: amplitude as a fraction of the canvas's height, waves in the height
// of the layer's box, the wave's direction in degrees, phase in waves.
// p[4], p[5]: the layer's box. p[6].xy: cosine and sine of the direction.
fn distort(at: vec2<f32>) -> vec4<f32> {
    let local = into_box(at, u.p[4].xy, u.p[5].xy);
    let along = dot(local, u.p[6].xy) / max(2.0 * u.p[4].w, 0.0001);
    let push = u.p[0].x * u.size.y * sin(TAU * (u.p[0].y * along + u.p[0].w));
    // The crests' direction, back in the canvas's axes.
    let crest = vec2<f32>(-u.p[6].y, u.p[6].x);
    let turned = vec2<f32>(
        crest.x * u.p[5].x - crest.y * u.p[5].y,
        crest.x * u.p[5].y + crest.y * u.p[5].x,
    );
    return pick(at + turned * push);
}

// Bands of rows pushed sideways and the colours pulled apart, drawn from
// the salt `pass.rs` made for this stretch of time.
//
// p[0]: amount, changes a second, the split as a fraction of the canvas's
// height. p[4].xy: the salt.
fn glitch(at: vec2<f32>) -> vec4<f32> {
    let amount = u.p[0].x;
    let salt = salt_of(u.p[4].xy);
    var shift = 0.0;
    let band = u32(at.y * 24.0 * u.size.w);
    if noise(vec2<u32>(band, 0u), salt) < 0.5 * amount {
        shift = (noise(vec2<u32>(band, 1u), salt) - 0.5) * 0.25 * amount * u.size.x;
    }
    let line = u32(at.y * 96.0 * u.size.w);
    if noise(vec2<u32>(line, 2u), salt) < 0.15 * amount {
        shift += (noise(vec2<u32>(line, 3u), salt) - 0.5) * 0.08 * amount * u.size.x;
    }
    let base = at + vec2<f32>(shift, 0.0);
    let split = vec2<f32>(u.p[0].z * u.size.y * amount, 0.0);
    let red = pick(base + split);
    let green = pick(base);
    let blue = pick(base - split);
    return vec4<f32>(red.r, green.g, blue.b, max(red.a, max(green.a, blue.a)));
}

// The mean of 24 samples a step apart along a line through the pixel: twice,
// with a long step and a short one, it is the mean along the whole streak.
//
// p[4].xy: the step in pixels.
fn streak(at: vec2<f32>) -> vec4<f32> {
    var sum = vec4<f32>(0.0);
    for (var i = 0; i < 24; i++) {
        let to = at + u.p[4].xy * (f32(i) - 11.5);
        sum += textureSampleLevel(first, smooth_clamp, to * u.size.zw, 0.0);
    }
    return sum / 24.0;
}

// The layer drawn small in the left top of the target: each pixel the mean
// of a square of the layer's, so that a wide blur costs a few samples.
//
// p[4]: the side of that square (x) and the size of the small picture (yz).
fn shrink(at: vec2<f32>) -> vec4<f32> {
    if at.x >= u.p[4].y || at.y >= u.p[4].z {
        return vec4<f32>(0.0);
    }
    let side = u32(u.p[4].x);
    let corner = floor(at) * u.p[4].x;
    // Each sample is the mean of the four texels it sits between.
    let n = max(side / 2u, 1u);
    var sum = vec4<f32>(0.0);
    for (var j = 0u; j < n; j++) {
        for (var i = 0u; i < n; i++) {
            let to = corner + vec2<f32>(f32(2u * i + 1u), f32(2u * j + 1u));
            sum += textureSampleLevel(first, smooth_clamp, to * u.size.zw, 0.0);
        }
    }
    return sum / f32(n * n);
}

// One direction of a gaussian blur over the small picture, whose edge texels
// stand for what is past them. `u.op.y`: 0 across, 1 down.
//
// p[4]: the size of the small picture (yz). p[5]: the deviation in its
// texels (x) and the texels taken either side (y).
fn gauss(at: vec2<f32>) -> vec4<f32> {
    if at.x >= u.p[4].y || at.y >= u.p[4].z {
        return vec4<f32>(0.0);
    }
    let here = vec2<i32>(at);
    let last = vec2<i32>(u.p[4].yz) - 1;
    let axis = select(vec2<i32>(1, 0), vec2<i32>(0, 1), u.op.y == 1u);
    let reach = i32(u.p[5].y);
    let spread = 2.0 * u.p[5].x * u.p[5].x;
    var sum = vec4<f32>(0.0);
    var weights = 0.0;
    for (var i = -reach; i <= reach; i++) {
        let weight = exp(-f32(i * i) / spread);
        sum += weight * textureLoad(first, clamp(here + axis * i, vec2<i32>(0), last), 0);
        weights += weight;
    }
    return sum / weights;
}

// The softened layer at a place on the canvas: the small picture, enlarged.
fn soft(at: vec2<f32>) -> vec4<f32> {
    let q = clamp(at / u.p[4].x, vec2<f32>(0.5), u.p[4].yz - 0.5);
    return textureSampleLevel(first, smooth_clamp, q * u.size.zw, 0.0);
}

// What is made of the layer and its softened self. `u.op.y` says which.
//
// first: the softened layer, small. second: the layer.
// p[0] on: the effect's parameters. p[4]: as `shrink`.
// p[6].xy: a shadow's offset in pixels.
fn softened(at: vec2<f32>, layer: vec4<f32>) -> vec4<f32> {
    switch u.op.y {
        case SOFT_GLOW: {
            // The light of what was over the threshold, added.
            let glow = soft(at) * u.p[0].x;
            let a = layer.a + (1.0 - layer.a) * clamp(glow.a, 0.0, 1.0);
            return vec4<f32>(min(layer.rgb + glow.rgb, vec3<f32>(a)), a);
        }
        case SOFT_SHARPEN: {
            // Unsharp mask: what the blur took away, given back over again.
            let detail = layer.rgb - soft(at).rgb;
            let rgb = clamp(layer.rgb + u.p[0].x * detail, vec3<f32>(0.0), vec3<f32>(layer.a));
            return vec4<f32>(rgb, layer.a);
        }
        case SOFT_SHADOW: {
            // The layer's softened alpha, black, under the layer.
            let shade = soft(at - u.p[6].xy).a * u.p[0].x;
            return vec4<f32>(layer.rgb, layer.a + shade * (1.0 - layer.a));
        }
        default: {
            return soft(at);
        }
    }
}

// What of the layer is brighter than the threshold, and by how much: the
// light a glow spreads.
//
// p[0].z: the threshold, in linear light.
fn bright(c: vec4<f32>) -> vec4<f32> {
    let light = dot(straight(c), LUMA);
    return c * (max(light - u.p[0].z, 0.0) / max(light, 0.0001));
}

// One point of the lookup table in `second`: a square tile for each step of
// blue, `u.op.y` of them in a row, with red across a tile and green down it.
fn lut_at(point: vec3<i32>) -> vec3<f32> {
    let n = i32(u.op.z);
    let across = i32(u.op.y);
    let tile = vec2<i32>(point.z % across, point.z / across);
    return textureLoad(second, tile * n + point.xy, 0).rgb;
}

// A colour looked up in a table of `u.op.z` points an edge. The table is of
// display colour, so the layer's linear light is gamma-encoded to look it
// up and what is found is decoded again. p[4] is the least input the table
// covers and p[5] one over the width of what it covers; an input outside
// that is looked up at its edge. The eight points around the colour are
// read as they are and mixed here, so no filtering blurs one tile into the
// next.
fn lut(c: vec3<f32>) -> vec3<f32> {
    let last = i32(u.op.z) - 1;
    let v = clamp((to_gamma(c) - u.p[4].xyz) * u.p[5].xyz, vec3<f32>(0.0), vec3<f32>(1.0))
        * f32(last);
    let low = clamp(vec3<i32>(floor(v)), vec3<i32>(0), vec3<i32>(max(last - 1, 0)));
    let high = min(low + vec3<i32>(1), vec3<i32>(last));
    let f = v - vec3<f32>(low);
    let near = mix(
        mix(lut_at(vec3<i32>(low.x, low.y, low.z)), lut_at(vec3<i32>(high.x, low.y, low.z)), f.x),
        mix(lut_at(vec3<i32>(low.x, high.y, low.z)), lut_at(vec3<i32>(high.x, high.y, low.z)), f.x),
        f.y,
    );
    let far = mix(
        mix(lut_at(vec3<i32>(low.x, low.y, high.z)), lut_at(vec3<i32>(high.x, low.y, high.z)), f.x),
        mix(lut_at(vec3<i32>(low.x, high.y, high.z)), lut_at(vec3<i32>(high.x, high.y, high.z)), f.x),
        f.y,
    );
    return to_linear(clamp(mix(near, far, f.z), vec3<f32>(0.0), vec3<f32>(1.0)));
}

@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let here = vec2<i32>(at.xy);
    var before: vec4<f32>;
    if u.op.w == 1u {
        before = textureLoad(second, here, 0);
    } else {
        before = textureLoad(first, here, 0);
    }
    var after = before;
    switch u.op.x {
        case OP_MASK: {
            after = before * mask_cover(at.xy);
        }
        case OP_GRADE: {
            after = coloured(before, grade(straight(before)));
        }
        case OP_HSL_BAND: {
            after = coloured(before, hsl_band(straight(before)));
        }
        case OP_CURVE: {
            after = coloured(before, curves(straight(before)));
        }
        case OP_CHROMA_KEY: {
            after = chroma_key(before);
        }
        case OP_DIFFERENCE_MATTE: {
            after = difference_matte(at.xy, before);
        }
        case OP_GRAIN: {
            after = coloured(before, grain(at.xy, straight(before)));
        }
        case OP_VIGNETTE: {
            after = vec4<f32>(before.rgb * vignette(at.xy), before.a);
        }
        case OP_SHAKE: {
            after = shake(at.xy);
        }
        case OP_DISTORT: {
            after = distort(at.xy);
        }
        case OP_GLITCH: {
            after = glitch(at.xy);
        }
        case OP_STREAK: {
            after = streak(at.xy);
        }
        case OP_SHRINK: {
            after = shrink(at.xy);
        }
        case OP_GAUSS: {
            after = gauss(at.xy);
        }
        case OP_SOFT: {
            after = softened(at.xy, before);
        }
        case OP_BRIGHT: {
            after = bright(before);
        }
        case OP_LUT: {
            after = coloured(before, lut(straight(before)));
        }
        default: {
        }
    }
    return mix(before, after, u.seed.z);
}
