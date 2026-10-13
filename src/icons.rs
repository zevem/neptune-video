//! Small, consistent vector icons drawn natively on a 24-point grid.
//!
//! One outline set with a 1.5-point stroke and rounded ends, matching
//! regular-weight text. An icon is recoloured for state, never swapped for
//! another asset. Transport glyphs are solid, as a player's are.

use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Play,
    Pause,
    SkipStart,
    SkipEnd,
    StepBack,
    StepForward,
    Loop,
    Scissors,
    Trash,
    Undo,
    Redo,
    Copy,
    Clipboard,
    Plus,
    Close,
    Check,
    ChevronDown,
    ChevronRight,
    ChevronLeft,
    Search,
    Folder,
    FolderPlus,
    Import,
    Save,
    Film,
    Image,
    Music,
    Text,
    Layers,
    Magnet,
    Flag,
    ZoomIn,
    ZoomOut,
    Fit,
    Eye,
    EyeOff,
    Lock,
    Unlock,
    Volume,
    VolumeOff,
    Sparkles,
    ArrowUp,
    ArrowDown,
    Settings,
    Sidebar,
    PanelRight,
    Grid,
    List,
    Warning,
    Link,
    Rotate,
    FlipHorizontal,
    FlipVertical,
    Pointer,
    File,
    History,
    Sort,
    Stop,
    Cube,
    Pipette,
    Diamond,
    Curve,
    FitValues,
    Transition,
}

fn arc(cx: f32, cy: f32, radius: f32, from: f32, to: f32, steps: usize) -> Vec<[f32; 2]> {
    (0..=steps)
        .map(|step| {
            let angle = (from + (to - from) * step as f32 / steps as f32).to_radians();
            [cx + radius * angle.cos(), cy + radius * angle.sin()]
        })
        .collect()
}

/// Paint an icon into its visual bounds. The caller controls the hit area.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let side = rect.width().min(rect.height());
    let origin = rect.center() - vec2(side, side) * 0.5;
    let scale = side / 24.0;
    let point = |x: f32, y: f32| origin + vec2(x, y) * scale;
    let stroke = Stroke::new(1.5, color);
    let line = |points: &[[f32; 2]]| {
        let points: Vec<Pos2> = points.iter().map(|p| point(p[0], p[1])).collect();
        let first = points.first().copied();
        let last = points.last().copied();
        painter.add(Shape::line(points, stroke));
        // egui's line primitives have square caps. These small discs give the
        // set rounded ends.
        if let Some(first) = first {
            painter.circle_filled(first, stroke.width * 0.5, color);
        }
        if let Some(last) = last {
            painter.circle_filled(last, stroke.width * 0.5, color);
        }
    };
    let rectangle = |x: f32, y: f32, width: f32, height: f32, radius: f32| {
        painter.rect_stroke(
            Rect::from_min_max(point(x, y), point(x + width, y + height)),
            (radius * scale).round() as u8,
            stroke,
            StrokeKind::Middle,
        );
    };
    let circle = |x: f32, y: f32, radius: f32| {
        painter.circle_stroke(point(x, y), radius * scale, stroke);
    };
    let dot = |x: f32, y: f32, radius: f32| {
        painter.circle_filled(point(x, y), radius * scale, color);
    };
    let solid = |points: &[[f32; 2]]| {
        painter.add(Shape::convex_polygon(
            points.iter().map(|p| point(p[0], p[1])).collect(),
            color,
            Stroke::new(1.0, color),
        ));
    };
    let bar = |x: f32, y: f32, width: f32, height: f32| {
        painter.rect_filled(
            Rect::from_min_max(point(x, y), point(x + width, y + height)),
            (1.2 * scale).round() as u8,
            color,
        );
    };

    match icon {
        // The triangle sits a little toward its point: its geometric centre
        // is left of where it looks centred.
        Icon::Play => solid(&[[8.5, 5.0], [19.5, 12.0], [8.5, 19.0]]),
        Icon::Pause => {
            bar(6.5, 5.0, 4.0, 14.0);
            bar(13.5, 5.0, 4.0, 14.0);
        }
        Icon::SkipStart => {
            bar(5.0, 6.0, 2.5, 12.0);
            solid(&[[19.0, 6.0], [9.5, 12.0], [19.0, 18.0]]);
        }
        Icon::SkipEnd => {
            bar(16.5, 6.0, 2.5, 12.0);
            solid(&[[5.0, 6.0], [14.5, 12.0], [5.0, 18.0]]);
        }
        Icon::StepBack => {
            line(&[[14.0, 6.0], [8.0, 12.0], [14.0, 18.0]]);
            line(&[[18.0, 7.0], [18.0, 17.0]]);
        }
        Icon::StepForward => {
            line(&[[10.0, 6.0], [16.0, 12.0], [10.0, 18.0]]);
            line(&[[6.0, 7.0], [6.0, 17.0]]);
        }
        Icon::Loop => {
            line(&[[7.0, 8.0], [17.0, 8.0], [20.0, 11.0], [20.0, 13.0]]);
            line(&[[14.5, 5.0], [17.5, 8.0], [14.5, 11.0]]);
            line(&[[17.0, 16.0], [7.0, 16.0], [4.0, 13.0], [4.0, 11.0]]);
            line(&[[9.5, 13.0], [6.5, 16.0], [9.5, 19.0]]);
        }
        Icon::Scissors => {
            circle(6.5, 7.0, 2.6);
            circle(6.5, 17.0, 2.6);
            line(&[[8.6, 8.5], [20.0, 17.5]]);
            line(&[[8.6, 15.5], [20.0, 6.5]]);
        }
        Icon::Trash => {
            line(&[[4.0, 7.0], [20.0, 7.0]]);
            line(&[[9.0, 7.0], [9.5, 4.5], [14.5, 4.5], [15.0, 7.0]]);
            line(&[
                [6.0, 7.0],
                [7.0, 19.0],
                [8.0, 20.0],
                [16.0, 20.0],
                [17.0, 19.0],
                [18.0, 7.0],
            ]);
            line(&[[10.0, 11.0], [10.0, 16.0]]);
            line(&[[14.0, 11.0], [14.0, 16.0]]);
        }
        Icon::Undo => {
            let mut path = vec![[5.0, 9.0], [14.0, 9.0]];
            path.extend(arc(14.0, 13.5, 4.5, -90.0, 90.0, 10));
            path.push([9.0, 18.0]);
            line(&path);
            line(&[[8.5, 5.5], [5.0, 9.0], [8.5, 12.5]]);
        }
        Icon::Redo => {
            let mut path = vec![[19.0, 9.0], [10.0, 9.0]];
            path.extend(arc(10.0, 13.5, 4.5, 270.0, 90.0, 10));
            path.push([15.0, 18.0]);
            line(&path);
            line(&[[15.5, 5.5], [19.0, 9.0], [15.5, 12.5]]);
        }
        Icon::Copy => {
            rectangle(8.0, 8.0, 12.0, 13.0, 2.5);
            line(&[
                [15.0, 5.0],
                [15.0, 3.0],
                [4.0, 3.0],
                [3.0, 4.0],
                [3.0, 15.0],
                [5.0, 15.0],
            ]);
        }
        Icon::Clipboard => {
            rectangle(5.0, 5.0, 14.0, 16.0, 2.5);
            rectangle(9.0, 3.0, 6.0, 4.0, 1.5);
            line(&[[9.0, 12.0], [15.0, 12.0]]);
            line(&[[9.0, 16.0], [13.0, 16.0]]);
        }
        Icon::Plus => {
            line(&[[12.0, 5.0], [12.0, 19.0]]);
            line(&[[5.0, 12.0], [19.0, 12.0]]);
        }
        Icon::Close => {
            line(&[[6.0, 6.0], [18.0, 18.0]]);
            line(&[[18.0, 6.0], [6.0, 18.0]]);
        }
        Icon::Check => line(&[[5.0, 12.0], [10.0, 17.0], [20.0, 7.0]]),
        Icon::ChevronDown => line(&[[6.0, 9.5], [12.0, 15.5], [18.0, 9.5]]),
        Icon::ChevronRight => line(&[[9.5, 6.0], [15.5, 12.0], [9.5, 18.0]]),
        Icon::ChevronLeft => line(&[[14.5, 6.0], [8.5, 12.0], [14.5, 18.0]]),
        Icon::Search => {
            circle(10.5, 10.5, 6.5);
            line(&[[15.3, 15.3], [20.0, 20.0]]);
        }
        Icon::Folder | Icon::FolderPlus => {
            line(&[
                [3.0, 7.0],
                [3.0, 18.0],
                [4.0, 19.0],
                [20.0, 19.0],
                [21.0, 18.0],
                [21.0, 9.0],
                [20.0, 8.0],
                [12.0, 8.0],
                [10.0, 5.0],
                [4.0, 5.0],
                [3.0, 6.0],
                [3.0, 7.0],
            ]);
            if icon == Icon::FolderPlus {
                line(&[[12.0, 11.0], [12.0, 16.0]]);
                line(&[[9.5, 13.5], [14.5, 13.5]]);
            }
        }
        Icon::Import => {
            line(&[[12.0, 4.0], [12.0, 15.0]]);
            line(&[[7.5, 10.5], [12.0, 15.0], [16.5, 10.5]]);
            line(&[
                [4.0, 15.0],
                [4.0, 19.0],
                [5.0, 20.0],
                [19.0, 20.0],
                [20.0, 19.0],
                [20.0, 15.0],
            ]);
        }
        Icon::Save => {
            line(&[
                [4.0, 5.0],
                [5.0, 4.0],
                [16.0, 4.0],
                [20.0, 8.0],
                [20.0, 19.0],
                [19.0, 20.0],
                [5.0, 20.0],
                [4.0, 19.0],
                [4.0, 5.0],
            ]);
            line(&[[8.0, 4.5], [8.0, 9.0], [15.0, 9.0], [15.0, 4.5]]);
            rectangle(8.0, 13.5, 8.0, 6.5, 1.0);
        }
        Icon::Film => {
            rectangle(3.0, 5.0, 18.0, 14.0, 3.0);
            line(&[[7.5, 5.5], [7.5, 18.5]]);
            line(&[[16.5, 5.5], [16.5, 18.5]]);
            for y in [9.5, 14.5] {
                line(&[[3.5, y], [7.5, y]]);
                line(&[[16.5, y], [20.5, y]]);
            }
        }
        Icon::Image => {
            rectangle(3.0, 5.0, 18.0, 14.0, 3.0);
            circle(8.5, 10.0, 1.6);
            line(&[
                [4.0, 17.0],
                [9.5, 12.5],
                [13.0, 15.5],
                [16.0, 13.0],
                [20.0, 16.5],
            ]);
        }
        Icon::Music => {
            // A waveform: what an audio clip looks like on the timeline.
            for (x, half) in [
                (4.0, 2.0),
                (8.0, 5.5),
                (12.0, 8.0),
                (16.0, 4.0),
                (20.0, 2.0),
            ] {
                line(&[[x, 12.0 - half], [x, 12.0 + half]]);
            }
        }
        Icon::Text => {
            line(&[[5.0, 7.5], [5.0, 5.0], [19.0, 5.0], [19.0, 7.5]]);
            line(&[[12.0, 5.0], [12.0, 19.0]]);
            line(&[[9.0, 19.0], [15.0, 19.0]]);
        }
        Icon::Layers => {
            line(&[
                [12.0, 4.0],
                [21.0, 9.0],
                [12.0, 14.0],
                [3.0, 9.0],
                [12.0, 4.0],
            ]);
            line(&[[3.0, 13.5], [12.0, 18.5], [21.0, 13.5]]);
        }
        Icon::Magnet => {
            let mut outer = vec![[5.0, 4.0], [5.0, 12.0]];
            outer.extend(arc(12.0, 12.0, 7.0, 180.0, 0.0, 14));
            outer.push([19.0, 4.0]);
            outer.push([15.0, 4.0]);
            outer.push([15.0, 12.0]);
            outer.extend(arc(12.0, 12.0, 3.0, 0.0, 180.0, 10));
            outer.push([9.0, 4.0]);
            outer.push([5.0, 4.0]);
            line(&outer);
            line(&[[5.0, 8.0], [9.0, 8.0]]);
            line(&[[15.0, 8.0], [19.0, 8.0]]);
        }
        Icon::Flag => {
            line(&[[6.0, 21.0], [6.0, 4.0]]);
            line(&[
                [6.0, 5.0],
                [18.0, 5.0],
                [15.0, 9.5],
                [18.0, 14.0],
                [6.0, 14.0],
            ]);
        }
        Icon::ZoomIn | Icon::ZoomOut => {
            circle(10.5, 10.5, 6.5);
            line(&[[15.3, 15.3], [20.0, 20.0]]);
            line(&[[7.5, 10.5], [13.5, 10.5]]);
            if icon == Icon::ZoomIn {
                line(&[[10.5, 7.5], [10.5, 13.5]]);
            }
        }
        Icon::Fit => {
            line(&[[4.0, 9.0], [4.0, 4.0], [9.0, 4.0]]);
            line(&[[15.0, 4.0], [20.0, 4.0], [20.0, 9.0]]);
            line(&[[20.0, 15.0], [20.0, 20.0], [15.0, 20.0]]);
            line(&[[9.0, 20.0], [4.0, 20.0], [4.0, 15.0]]);
        }
        Icon::Eye | Icon::EyeOff => {
            let mut lid = arc(12.0, 17.5, 11.0, 215.0, 325.0, 14);
            lid.extend(arc(12.0, 6.5, 11.0, 35.0, 145.0, 14));
            lid.push(lid[0]);
            line(&lid);
            circle(12.0, 12.0, 2.8);
            if icon == Icon::EyeOff {
                line(&[[4.5, 4.5], [19.5, 19.5]]);
            }
        }
        Icon::Lock | Icon::Unlock => {
            rectangle(5.0, 10.5, 14.0, 10.0, 2.5);
            let mut shackle = vec![[8.0, 10.5], [8.0, 8.0]];
            shackle.extend(arc(12.0, 8.0, 4.0, 180.0, 360.0, 12));
            if icon == Icon::Lock {
                shackle.push([16.0, 10.5]);
            }
            line(&shackle);
        }
        Icon::Volume | Icon::VolumeOff => {
            line(&[
                [3.5, 9.5],
                [7.0, 9.5],
                [11.5, 5.5],
                [11.5, 18.5],
                [7.0, 14.5],
                [3.5, 14.5],
                [3.5, 9.5],
            ]);
            if icon == Icon::Volume {
                line(&arc(11.5, 12.0, 4.0, -45.0, 45.0, 8));
                line(&arc(11.5, 12.0, 7.5, -45.0, 45.0, 10));
            } else {
                line(&[[15.0, 9.0], [21.0, 15.0]]);
                line(&[[21.0, 9.0], [15.0, 15.0]]);
            }
        }
        Icon::Sparkles => {
            let star = |cx: f32, cy: f32, radius: f32| {
                let inner = radius * 0.32;
                let mut points = Vec::with_capacity(9);
                for step in 0..8 {
                    let angle = (step as f32 * 45.0 - 90.0).to_radians();
                    let r = if step % 2 == 0 { radius } else { inner };
                    points.push([cx + r * angle.cos(), cy + r * angle.sin()]);
                }
                points.push(points[0]);
                points
            };
            line(&star(10.0, 13.0, 7.5));
            line(&star(18.5, 6.0, 3.2));
        }
        Icon::ArrowUp => {
            line(&[[12.0, 19.0], [12.0, 5.0]]);
            line(&[[6.0, 11.0], [12.0, 5.0], [18.0, 11.0]]);
        }
        Icon::ArrowDown => {
            line(&[[12.0, 5.0], [12.0, 19.0]]);
            line(&[[6.0, 13.0], [12.0, 19.0], [18.0, 13.0]]);
        }
        Icon::Settings => {
            let mut outline = Vec::with_capacity(33);
            for tooth in 0..8 {
                for (offset, radius) in [(0.0, 7.5), (0.2, 9.5), (0.55, 9.5), (0.75, 7.5)] {
                    let angle = (tooth as f32 + offset) * std::f32::consts::TAU / 8.0;
                    outline.push([12.0 + radius * angle.cos(), 12.0 + radius * angle.sin()]);
                }
            }
            outline.push(outline[0]);
            line(&outline);
            circle(12.0, 12.0, 3.0);
        }
        Icon::Sidebar => {
            rectangle(3.0, 4.0, 18.0, 16.0, 3.0);
            line(&[[8.5, 4.5], [8.5, 19.5]]);
        }
        Icon::PanelRight => {
            rectangle(3.0, 4.0, 18.0, 16.0, 3.0);
            line(&[[15.5, 4.5], [15.5, 19.5]]);
        }
        Icon::Grid => {
            for y in [4.0, 14.0] {
                for x in [4.0, 14.0] {
                    rectangle(x, y, 6.0, 6.0, 1.5);
                }
            }
        }
        Icon::List => {
            for y in [6.5, 12.0, 17.5] {
                dot(4.5, y, 1.1);
                line(&[[8.5, y], [20.0, y]]);
            }
        }
        Icon::Link => {
            line(&[[9.5, 14.5], [14.5, 9.5]]);
            line(&[
                [11.0, 6.5],
                [12.5, 5.0],
                [15.5, 4.2],
                [18.6, 5.4],
                [19.8, 8.5],
                [19.0, 11.5],
                [17.5, 13.0],
            ]);
            line(&[
                [13.0, 17.5],
                [11.5, 19.0],
                [8.5, 19.8],
                [5.4, 18.6],
                [4.2, 15.5],
                [5.0, 12.5],
                [6.5, 11.0],
            ]);
        }
        Icon::Warning => {
            line(&[[12.0, 3.5], [21.5, 20.0], [2.5, 20.0], [12.0, 3.5]]);
            line(&[[12.0, 9.5], [12.0, 14.0]]);
            dot(12.0, 17.0, 1.0);
        }
        Icon::Rotate => {
            line(&arc(12.0, 12.0, 8.0, -60.0, 240.0, 24));
            line(&[[16.0, 2.5], [16.2, 5.6], [19.4, 5.2]]);
        }
        Icon::FlipHorizontal => {
            line(&[[12.0, 3.5], [12.0, 20.5]]);
            line(&[[9.0, 7.0], [3.5, 17.0], [9.0, 17.0], [9.0, 7.0]]);
            line(&[[15.0, 7.0], [20.5, 17.0], [15.0, 17.0], [15.0, 7.0]]);
        }
        Icon::FlipVertical => {
            line(&[[3.5, 12.0], [20.5, 12.0]]);
            line(&[[7.0, 9.0], [17.0, 3.5], [17.0, 9.0], [7.0, 9.0]]);
            line(&[[7.0, 15.0], [17.0, 20.5], [17.0, 15.0], [7.0, 15.0]]);
        }
        Icon::Pointer => {
            line(&[
                [5.0, 3.5],
                [5.0, 18.5],
                [9.2, 14.6],
                [12.0, 20.5],
                [14.4, 19.4],
                [11.7, 13.6],
                [17.5, 13.6],
                [5.0, 3.5],
            ]);
        }
        Icon::File => {
            line(&[
                [6.0, 4.0],
                [7.0, 3.0],
                [14.0, 3.0],
                [19.0, 8.0],
                [19.0, 20.0],
                [18.0, 21.0],
                [7.0, 21.0],
                [6.0, 20.0],
                [6.0, 4.0],
            ]);
            line(&[[14.0, 3.5], [14.0, 8.0], [18.5, 8.0]]);
        }
        Icon::History => {
            let mut path = vec![[3.5, 3.5], [3.5, 8.0]];
            path.extend(arc(12.0, 12.0, 8.5, -132.0, 174.0, 24));
            line(&path);
            line(&[[3.5, 8.0], [8.0, 8.0]]);
            line(&[[12.0, 7.5], [12.0, 12.0], [15.5, 14.0]]);
        }
        Icon::Sort => {
            line(&[[8.0, 19.0], [8.0, 5.0]]);
            line(&[[4.5, 8.5], [8.0, 5.0], [11.5, 8.5]]);
            line(&[[16.0, 5.0], [16.0, 19.0]]);
            line(&[[12.5, 15.5], [16.0, 19.0], [19.5, 15.5]]);
        }
        Icon::Stop => bar(6.5, 6.5, 11.0, 11.0),
        Icon::Cube => {
            line(&[
                [12.0, 3.0],
                [20.0, 7.5],
                [20.0, 16.5],
                [12.0, 21.0],
                [4.0, 16.5],
                [4.0, 7.5],
                [12.0, 3.0],
            ]);
            line(&[[4.0, 7.5], [12.0, 12.0], [20.0, 7.5]]);
            line(&[[12.0, 12.0], [12.0, 21.0]]);
        }
        Icon::Pipette => {
            // The bulb, the collar under it and the barrel down to the tip.
            line(&[
                [14.5, 6.0],
                [16.5, 4.0],
                [20.0, 7.5],
                [18.0, 9.5],
                [14.5, 6.0],
            ]);
            line(&[[12.5, 6.5], [17.5, 11.5]]);
            line(&[
                [14.0, 8.0],
                [5.5, 16.5],
                [4.5, 19.5],
                [7.5, 18.5],
                [16.0, 10.0],
            ]);
        }
        Icon::Diamond => {
            line(&[
                [12.0, 4.0],
                [20.0, 12.0],
                [12.0, 20.0],
                [4.0, 12.0],
                [12.0, 4.0],
            ]);
        }
        Icon::Curve => {
            // An eased curve between two keys.
            let curve: Vec<[f32; 2]> = (0..=12)
                .map(|step| {
                    let t = step as f32 / 12.0;
                    let r = 1.0 - t;
                    let x = 3.0 * r * t * t * 7.0 + 3.0 * r * r * t * 9.0 + t * t * t * 16.0;
                    [4.0 + x, 18.0 - 12.0 * t * t * (3.0 - 2.0 * t)]
                })
                .collect();
            line(&curve);
            dot(4.0, 18.0, 2.2);
            dot(20.0, 6.0, 2.2);
        }
        Icon::FitValues => {
            line(&[[5.0, 4.0], [19.0, 4.0]]);
            line(&[[5.0, 20.0], [19.0, 20.0]]);
            line(&[[12.0, 7.5], [12.0, 16.5]]);
            line(&[[9.0, 10.0], [12.0, 7.5], [15.0, 10.0]]);
            line(&[[9.0, 14.0], [12.0, 16.5], [15.0, 14.0]]);
        }
        Icon::Transition => {
            // Two frames, one giving way to the other across a diagonal.
            rectangle(3.5, 6.0, 17.0, 12.0, 2.5);
            line(&[[8.5, 18.0], [15.5, 6.0]]);
        }
    }
}
