// The radar's marker layer, composited as ONE image.
//
// In flight the map markers (dots, icon miniatures, their halos, the
// guide ants and objective marks) are drawn first into an offscreen
// layer the size of the scene, cleared transparent, with PREMULTIPLIED
// accumulation: overlapping quads settle there exactly as they would
// opaque on screen, so a miniature fully hides the halo silhouettes
// stacked beneath it. This pass then lays that finished layer over the
// radar and applies, to the layer as a whole, what the radar disc
// itself gets:
//
// - the HUD transparency (one alpha for the layer — fading each quad
//   on its own let the halo silhouettes under every miniature show
//   through: the "hazy" icons);
// - the level-end fade, as DARKENING (the fade quad below already
//   blackened what the layer covers; scaling the layer's colour by the
//   same `keep` lands exactly on "everything under one black quad");
// - the death greyscale, the same sRGB `(r+g+b)/3` mix the world
//   shaders run (`grade.x`).

struct LayerGlobals {
    // x, y, w, h of the composited rect, in pixels (the radar's
    // bounding square — the markers are clipped to it).
    rect: vec4<f32>,
    // xy = scene size in pixels.
    screen: vec4<f32>,
    // x = alpha (HUD transparency), y = keep (1 - level-end fade),
    // z = greyscale amount.
    fx: vec4<f32>,
};

@group(0) @binding(0) var<uniform> lg: LayerGlobals;
@group(0) @binding(1) var t_layer: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let px = lg.rect.xy + corners[vid] * lg.rect.zw;
    return vec4<f32>(
        px.x / lg.screen.x * 2.0 - 1.0,
        1.0 - px.y / lg.screen.y * 2.0,
        0.0,
        1.0,
    );
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3<f32>(0.0031308));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    // The layer is the scene's size: one texel per pixel.
    let c = textureLoad(t_layer, vec2<i32>(pos.xy), 0);
    if c.a <= 0.0 {
        discard;
    }
    var rgb = c.rgb;
    let g = lg.fx.z;
    if g > 0.0 {
        var s = to_srgb(rgb / c.a);
        s = mix(s, vec3<f32>((s.r + s.g + s.b) / 3.0), g);
        rgb = to_linear(s) * c.a;
    }
    // Premultiplied out: colour darkened by the fade, both scaled by
    // the layer alpha.
    let a = lg.fx.x;
    return vec4<f32>(rgb * a * lg.fx.y, c.a * a);
}
