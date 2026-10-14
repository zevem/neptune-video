//! What a frame is assembled from, without a device: the boxes of media and
//! titles, and the canvases a whole frame is stacked on.
use super::{
    Draw, Job,
    assemble::{self, LayerBox},
    gpu_tests::{NIGHT, Pic, RED},
    stack_passes, stack_targets,
};
use crate::textures::Input;
use editor_core::{BlendMode, Layer, LayerSource, RenderPlan, Time, Transform};

fn graphic(layer: Layer) -> Layer {
    Layer {
        source: LayerSource::Graphic {
            size: [400.0, 120.0],
            elements: Vec::new(),
        },
        ..layer
    }
}

#[test]
fn a_frame_is_one_canvas_but_for_a_run_drawn_over_a_title() {
    let pics = [
        Pic::full(RED).blend(BlendMode::Screen),
        Pic::full(RED),
        Pic::full(RED),
        Pic::full(RED).blend(BlendMode::Multiply),
        Pic::full(RED),
    ];
    let mut layers: Vec<Layer> = pics.iter().map(|pic| pic.layer.clone()).collect();
    layers[1] = graphic(layers[1].clone());
    let plan = RenderPlan {
        canvas: [64, 64],
        background: NIGHT,
        layers,
    };
    let draws = (plan.layers.iter().zip(&pics))
        .map(|(layer, pic)| {
            Some(Draw {
                layer,
                place: pic.place,
                input: Input {
                    serial: pic.serial,
                    frame: &pic.frame,
                    time: Time::ZERO,
                },
                plates: Vec::new(),
                luts: Vec::new(),
                steady: None,
            })
        })
        .collect();
    let jobs = assemble::stack(&plan, draws);
    let clips = |job: &Job<'_>| {
        job.draws
            .iter()
            .map(|draw| draw.layer.clip)
            .collect::<Vec<_>>()
    };
    let clip = |index: usize| plan.layers[index].clip;
    assert_eq!(
        jobs.len(),
        2,
        "the run over the title has a canvas of its own"
    );
    assert_eq!(jobs[0].background, Some(NIGHT));
    assert_eq!(clips(&jobs[0]), vec![clip(0), clip(1)]);
    assert_eq!(jobs[1].background, None, "it starts clear");
    assert_eq!(
        clips(&jobs[1]),
        vec![clip(2), clip(3), clip(4)],
        "the plain layer over it is laid over its canvas"
    );
    // Each canvas's passes, the second laid over the first in the pass
    // that would have encoded it.
    assert_eq!(stack_passes(&jobs), jobs[0].passes() + jobs[1].passes());
    assert_eq!(
        stack_targets(&jobs),
        4 + 1,
        "a canvas for the run, and one a layer"
    );
    assert_eq!(
        stack_targets(&jobs[..1]),
        2 + 1,
        "one canvas needs no second"
    );
}

#[test]
fn a_frame_with_nothing_on_it_is_its_canvas_colour() {
    let plan = RenderPlan {
        canvas: [64, 64],
        background: NIGHT,
        layers: Vec::new(),
    };
    let jobs = assemble::stack(&plan, Vec::new());
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].background, Some(NIGHT));
    assert!(jobs[0].draws.is_empty());
    assert_eq!(stack_passes(&jobs), 1, "the encoding alone");
}

#[test]
fn a_layer_box_is_its_fitted_picture_by_the_clip_scale() {
    let transform = Transform {
        position: [100.0, -50.0],
        scale: [0.5, 0.25],
        rotation: 90.0,
        ..Transform::default()
    };
    let canvas = [1920.0, 1080.0];
    let layer_box = assemble::layer_box(&transform, [4000.0, 1000.0], canvas);
    assert_eq!(
        layer_box,
        LayerBox {
            position: [100.0, -50.0],
            size: [960.0, 120.0],
        }
    );
    let place = layer_box.placement(canvas, transform.rotation);
    assert_eq!(place.center, [1060.0, 490.0]);
    assert_eq!(place.half, [480.0, 60.0]);
    assert!((place.angle - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
}

#[test]
fn a_title_box_has_the_corners_the_viewer_draws_its_quad_at() {
    let canvas = [640.0, 360.0];
    let (origin, size) = ([-30.0, -8.0], [70.0, 20.0]);
    for transform in [
        Transform::default(),
        Transform {
            position: [40.0, -20.0],
            scale: [2.0, 0.5],
            rotation: 30.0,
            ..Transform::default()
        },
        Transform {
            position: [-10.0, 15.0],
            scale: [1.5, 1.5],
            rotation: -120.0,
            flip_x: true,
            flip_y: true,
            ..Transform::default()
        },
    ] {
        let anchor = [
            canvas[0] * 0.5 + transform.position[0],
            canvas[1] * 0.5 + transform.position[1],
        ];
        let corner = |u: f32, v: f32| {
            assemble::title_point(
                anchor,
                1.0,
                &transform,
                origin[0] + u * size[0],
                origin[1] + v * size[1],
            )
        };
        let corners = [
            corner(0.0, 0.0),
            corner(1.0, 0.0),
            corner(1.0, 1.0),
            corner(0.0, 1.0),
        ];
        let place = assemble::title_placement(canvas, &transform, origin, size);
        let mean = |axis: usize| corners.iter().map(|point| point[axis]).sum::<f32>() / 4.0;
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        assert!(
            near(place.center[0], mean(0)) && near(place.center[1], mean(1)),
            "{transform:?}: the box's centre {:?} is the quad's",
            place.center
        );
        let length = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
        assert!(near(place.half[0] * 2.0, length(corners[0], corners[1])));
        assert!(near(place.half[1] * 2.0, length(corners[1], corners[2])));
    }
}

#[test]
fn a_title_is_drawn_faded_through_its_transition_with_nothing_else() {
    let mut layer = Pic::full(RED).blend(BlendMode::Screen).layer;
    layer.transform.opacity = 0.8;
    layer.transform.crop = [0.1, 0.0, 0.0, 0.0];
    layer.transition = Some(editor_core::ResolvedTransition {
        kind: editor_core::TransitionKind::Dissolve,
        progress: 0.25,
        outgoing: false,
        partner: None,
        params: Vec::new(),
    });
    let drawn = assemble::title_layer(&layer);
    assert!(
        (drawn.transform.opacity - 0.2).abs() < 1e-6,
        "faded in a quarter"
    );
    assert_eq!(drawn.transform.crop, [0.0; 4], "a title is never cropped");
    assert_eq!(drawn.blend, BlendMode::Normal);
    assert!(drawn.transition.is_none() && drawn.effects.is_empty() && drawn.masks.is_empty());
    assert!(drawn.is_plain());
}
