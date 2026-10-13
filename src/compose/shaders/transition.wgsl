// The transition pass: the clip that is over and the clip it leads into,
// made one layer. That layer is then blended as any other is.
//
// first: the outgoing layer, placed and with its chain drawn.
// second: the incoming layer, likewise. A side that is not there is blank:
// a clip with no partner comes from nothing or goes to it, and so does one
// whose partner has no picture yet.
//
// u.op.x: the kind, `KIND_*` below. u.seed.xy: the noise of this frame.
// p[0]: progress (x), the kind's parameter (y), and the opacity of the
// outgoing (z) and the incoming (w) layer.
// p[1]: the way a slide or a wipe moves (xy), one of the four axes, and the
// scale a zoom draws the outgoing (z) and the incoming (w) layer at.
// p[2]: the centre of the outgoing (xy) and the incoming (zw) layer's box,
// in target pixels: what each is zoomed about.
//
// Progress is eased before it comes here. At nought every kind is the
// outgoing layer alone and at one the incoming layer alone. Slides and
// wipes cross the canvas, not the layers' boxes.

// A mix of the two in linear light: (1 - p) of one and p of the other. A
// blur is this over two layers that were blurred before they came.
const KIND_DISSOLVE: u32 = 1u;
// A fade between two clips: the outgoing fades to nothing over the first
// half and the incoming from nothing over the second, so half way there is
// neither, and what is under the layer shows.
const KIND_DIP: u32 = 2u;
// The incoming pushes the outgoing off the canvas, edge to edge.
const KIND_SLIDE: u32 = 3u;
// Both scaled as one picture pushed in to or drawn back from, and mixed.
const KIND_ZOOM: u32 = 4u;
// An edge crosses the canvas with the incoming behind it. p[0].y is the
// width of its soft band as a fraction of the canvas along the way it goes.
const KIND_WIPE: u32 = 5u;
// Bands of rows cut from one to the other, torn sideways and their colours
// pulled apart, most half way. p[0].y is how much.
const KIND_GLITCH: u32 = 6u;

// The outgoing layer at a place between its pixels, and nothing past the
// canvas.
fn leaving(at: vec2<f32>) -> vec4<f32> {
    let uv = at * u.size.zw;
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) {
        return vec4<f32>(0.0);
    }
    return textureSampleLevel(first, smooth_clamp, uv, 0.0) * u.p[0].z;
}

// The incoming layer, likewise.
fn arriving(at: vec2<f32>) -> vec4<f32> {
    let uv = at * u.size.zw;
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) {
        return vec4<f32>(0.0);
    }
    return textureSampleLevel(second, smooth_clamp, uv, 0.0) * u.p[0].w;
}

// One of the two, with the canvas's edge standing for what is past it: a
// torn row of a picture that fills the canvas brings in no clear canvas.
fn either(at: vec2<f32>, incoming: bool) -> vec4<f32> {
    let uv = at * u.size.zw;
    if incoming {
        return textureSampleLevel(second, smooth_clamp, uv, 0.0) * u.p[0].w;
    }
    return textureSampleLevel(first, smooth_clamp, uv, 0.0) * u.p[0].z;
}

// Each of 24 bands of rows shows the incoming layer once progress passes
// the number drawn for it, so more of them do as it goes on. How far rows
// are torn and colours parted rises to the middle and falls back to
// nothing, so both ends are a whole picture.
fn glitched(at: vec2<f32>) -> vec4<f32> {
    let progress = u.p[0].x;
    let amount = u.p[0].y * (1.0 - abs(2.0 * progress - 1.0));
    let salt = salt_of(u.seed.xy);
    let band = u32(at.y * 24.0 * u.size.w);
    var shift = 0.0;
    if noise(vec2<u32>(band, 0u), salt) < amount {
        shift = (noise(vec2<u32>(band, 1u), salt) - 0.5) * 0.25 * amount * u.size.x;
    }
    let line = u32(at.y * 96.0 * u.size.w);
    if noise(vec2<u32>(line, 2u), salt) < 0.3 * amount {
        shift += (noise(vec2<u32>(line, 3u), salt) - 0.5) * 0.08 * amount * u.size.x;
    }
    let incoming = noise(vec2<u32>(band, 4u), salt) < progress;
    let base = at + vec2<f32>(shift, 0.0);
    let split = vec2<f32>(0.02 * amount * u.size.y, 0.0);
    let red = either(base + split, incoming);
    let green = either(base, incoming);
    let blue = either(base - split, incoming);
    return vec4<f32>(red.r, green.g, blue.b, max(red.a, max(green.a, blue.a)));
}

@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let progress = u.p[0].x;
    switch u.op.x {
        case KIND_DIP: {
            return leaving(at.xy) * max(1.0 - 2.0 * progress, 0.0)
                + arriving(at.xy) * max(2.0 * progress - 1.0, 0.0);
        }
        case KIND_SLIDE: {
            // By whole pixels, so neither picture is softened on its way.
            let whole = u.p[1].xy * u.size.xy;
            let moved = round(whole * progress);
            return leaving(at.xy - moved) + arriving(at.xy - moved + whole);
        }
        case KIND_ZOOM: {
            let going = leaving(u.p[2].xy + (at.xy - u.p[2].xy) / max(u.p[1].z, 0.0001));
            let coming = arriving(u.p[2].zw + (at.xy - u.p[2].zw) / max(u.p[1].w, 0.0001));
            return mix(going, coming, progress);
        }
        case KIND_WIPE: {
            // How far the pixel is along the way the edge goes: nought where
            // it starts and one where it ends.
            let along = dot(at.xy * u.size.zw - 0.5, u.p[1].xy) + 0.5;
            // An edge that is not soft is one pixel wide.
            let soft = max(u.p[0].y, abs(dot(u.p[1].xy, u.size.zw)));
            let ramp = clamp((progress * (1.0 + soft) - along) / soft, 0.0, 1.0);
            return mix(leaving(at.xy), arriving(at.xy), ramp * ramp * (3.0 - 2.0 * ramp));
        }
        case KIND_GLITCH: {
            return glitched(at.xy);
        }
        default: {
            return mix(leaving(at.xy), arriving(at.xy), progress);
        }
    }
}
