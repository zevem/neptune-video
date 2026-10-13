// The blend pass: a placed layer over what is under it.
//
// first: the backdrop. second: the layer. Both premultiplied linear light.
// u.op.y: the blend mode, in the order of `editor_core::BlendMode`.
// u.seed.w: the layer's opacity.
//
// The modes are those of the W3C compositing specification, in linear light:
// where the backdrop is clear the layer shows as it is.

fn soft_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        return b - (1.0 - 2.0 * s) * b * (1.0 - b);
    }
    var d: f32;
    if b <= 0.25 {
        d = ((16.0 * b - 12.0) * b + 4.0) * b;
    } else {
        d = sqrt(b);
    }
    return b + (2.0 * s - 1.0) * (d - b);
}

fn blend(b: vec3<f32>, s: vec3<f32>, mode: u32) -> vec3<f32> {
    switch mode {
        case 1u: {
            return b * s;
        }
        case 2u: {
            return b + s - b * s;
        }
        case 3u: {
            return select(1.0 - 2.0 * (1.0 - b) * (1.0 - s), 2.0 * b * s, b <= vec3<f32>(0.5));
        }
        case 4u: {
            return vec3<f32>(soft_light(b.r, s.r), soft_light(b.g, s.g), soft_light(b.b, s.b));
        }
        case 5u: {
            return min(b + s, vec3<f32>(1.0));
        }
        case 6u: {
            return min(b, s);
        }
        case 7u: {
            return max(b, s);
        }
        case 8u: {
            return abs(b - s);
        }
        default: {
            return s;
        }
    }
}

@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let b = textureLoad(first, vec2<i32>(at.xy), 0);
    let s = textureLoad(second, vec2<i32>(at.xy), 0) * u.seed.w;
    if s.a <= 0.0 {
        return b;
    }
    var colour = s.rgb + b.rgb * (1.0 - s.a);
    if u.op.y != 0u && b.a > 0.0 {
        let under = clamp(b.rgb / b.a, vec3<f32>(0.0), vec3<f32>(1.0));
        let over = clamp(s.rgb / s.a, vec3<f32>(0.0), vec3<f32>(1.0));
        colour = s.rgb * (1.0 - b.a) + b.rgb * (1.0 - s.a) + s.a * b.a * blend(under, over, u.op.y);
    }
    return vec4<f32>(colour, s.a + b.a - s.a * b.a);
}
