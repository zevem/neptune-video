//! Effects, masks, blend modes, looks and transitions; keyframes and speed
//! changes over time. Built as the window's inspector and browser build
//! them; the core checks every value against the same ranges.
use super::{Guard, batch, clip, refused};
use crate::{
    mcp::{errors::ToolError, server::NeptuneVideoServer, time::TimeSpec, workspace::Edited},
    speed::{self, Curve, Ramp},
    ui::{inspector, transitions},
};
use editor_core::{
    AnimTarget, BlendMode, Command, Ease, Effect, EffectKind, Keyframe, LookPreset, Mask, MaskOp,
    MaskShape, ParamKey, ParamValue, Project, TextAnimationKind, Time, TrackKind, TransitionEdge,
    TransitionKind,
    look::{self, ParamSpec},
};
use rmcp::{Json, handler::server::wrapper::Parameters, tool, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, de::DeserializeOwned};
use std::collections::BTreeMap;
use uuid::Uuid;

/// A name from the core's snake_case vocabulary.
pub fn named<T: DeserializeOwned>(what: &str, text: &str) -> Result<T, ToolError> {
    serde_json::from_value(serde_json::Value::String(text.trim().to_owned())).map_err(|_| {
        ToolError::invalid(format!(
            "“{text}” is not a {what}; effects_catalog lists them"
        ))
    })
}

/// The names a core enum takes, from its schema.
fn names_of(schema: serde_json::Value) -> serde_json::Value {
    let mut names = Vec::new();
    let mut take = |value: &serde_json::Value| {
        if let Some(list) = value["enum"].as_array() {
            names.extend(list.iter().cloned());
        }
        if let Some(name) = value.get("const") {
            names.push(name.clone());
        }
    };
    take(&schema);
    for key in ["oneOf", "anyOf"] {
        for variant in schema[key].as_array().into_iter().flatten() {
            take(variant);
        }
    }
    names.into()
}

fn blend_modes() -> serde_json::Value {
    names_of(serde_json::to_value(schemars::schema_for!(BlendMode)).unwrap_or_default())
}

fn name_of<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn specs(target: AnimTarget, specs: &[ParamSpec]) -> Vec<serde_json::Value> {
    specs
        .iter()
        .map(|spec| {
            let (scale, unit) = inspector::param_unit(target, spec);
            serde_json::json!({
                "key": spec.key,
                "name": inspector::param_name(target, spec.key),
                "min": spec.min,
                "max": spec.max,
                "default": spec.default,
                "shown_as": {"multiplier": scale, "unit": unit},
                "animatable": look::animatable(target, spec.key),
            })
        })
        .collect()
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EffectParams {
    /// `add`, `update`, `remove`, `masks`, `blend` or `look`.
    pub op: String,
    /// The clips (`add`, `masks`, `blend`, `look`) or the one clip
    /// (`update`, `remove`).
    pub clip_ids: Vec<Uuid>,
    /// `add`: an effect kind from effects_catalog.
    #[serde(default)]
    pub kind: Option<String>,
    /// `add` on audio clips: `voice` or `three_band_eq` adds a ready chain.
    #[serde(default)]
    pub preset: Option<String>,
    /// `update`, `remove`: the effect.
    #[serde(default)]
    pub effect_id: Option<Uuid>,
    /// `add`, `update`: parameter values by key, in stored units.
    #[serde(default)]
    pub params: BTreeMap<String, f32>,
    /// `add`, `update`: how much of the effect shows, 0 to 1 (picture only).
    #[serde(default)]
    pub mix: Option<f32>,
    #[serde(default)]
    pub enabled: Option<bool>,
    /// `add`, `update`: the LUT (for lut) or still (for difference_matte).
    #[serde(default)]
    pub asset_id: Option<String>,
    /// `add`, `update` of a curve effect: its points, x and y from 0 to 1.
    #[serde(default)]
    pub points: Option<Vec<[f32; 2]>>,
    /// `masks`: the clip's masks after the edit, in order.
    #[serde(default)]
    pub masks: Option<Vec<MaskSpec>>,
    /// `blend`: a blend mode from effects_catalog.
    #[serde(default)]
    pub mode: Option<String>,
    /// `look`: a look from effects_catalog, or `none` to take it off.
    #[serde(default)]
    pub look: Option<String>,
    /// `look`: 0.01 to 1.
    #[serde(default)]
    pub strength: Option<f32>,
    #[serde(flatten)]
    pub guard: Guard,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MaskSpec {
    /// Keep a mask the clip has (and its keyframes) by its ID.
    #[serde(default)]
    pub mask_id: Option<Uuid>,
    /// `rectangle` or `ellipse`.
    #[serde(default)]
    pub shape: Option<MaskShape>,
    /// `add`, `subtract` or `intersect`.
    #[serde(default)]
    pub op: Option<MaskOp>,
    #[serde(default)]
    pub invert: Option<bool>,
    /// The centre, 0 to 1 across and down the clip's picture.
    #[serde(default)]
    pub center: Option<[f32; 2]>,
    /// Width and height, 0 to 1 of the picture.
    #[serde(default)]
    pub size: Option<[f32; 2]>,
    #[serde(default)]
    pub rotation: Option<f32>,
    #[serde(default)]
    pub feather: Option<f32>,
    #[serde(default)]
    pub roundness: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TransitionParams {
    pub clip_id: Uuid,
    /// `in` (the clip's start) or `out` (its end, which is the next clip's
    /// start when one follows).
    pub edge: TransitionEdge,
    /// A transition kind from effects_catalog, or `none` to remove it.
    pub kind: String,
    /// Half a second by default; at most 10 s and the clip's length.
    #[serde(default)]
    pub duration: Option<TimeSpec>,
    #[serde(flatten)]
    pub guard: Guard,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KeyParams {
    pub clip_id: Uuid,
    /// The parameter key from effects_catalog: a clip key (position_x,
    /// scale_x, rotation, opacity, gain, crop_left…), or an effect's or
    /// mask's.
    pub key: String,
    /// The effect or mask the key belongs to; none for the clip's own.
    #[serde(default)]
    pub target_id: Option<Uuid>,
    /// The keyframes. Their `at` is a timeline time inside the clip.
    #[serde(default)]
    pub keys: Vec<KeySpec>,
    /// `merge` (the default) adds to the keyframes there, replacing one at
    /// the same time; `replace` sets exactly these (none removes the
    /// animation).
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(flatten)]
    pub guard: Guard,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct KeySpec {
    pub at: TimeSpec,
    pub value: f32,
    /// How the value moves to the next key: `linear` (the default),
    /// `smooth`, `hold`, or a cubic bezier [x1, y1, x2, y2].
    #[serde(default)]
    pub ease: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RetimeParams {
    pub clip_id: Uuid,
    /// `ramp` (a named speed ramp), `reverse`, `freeze` (hold a frame),
    /// `pitch` (shift the sound's pitch), `keep_pitch`, or `reset` (speed
    /// 1, no map).
    pub op: String,
    /// `ramp`: ramp_up, ramp_down, montage, hero, bullet, flash_in or
    /// flash_out.
    #[serde(default)]
    pub ramp: Option<String>,
    /// `freeze`: the timeline time of the frame to hold.
    #[serde(default)]
    pub at: Option<TimeSpec>,
    /// `freeze`: how long to hold it, 2 s by default.
    #[serde(default)]
    pub hold: Option<TimeSpec>,
    /// `pitch`: semitones, -12 to 12.
    #[serde(default)]
    pub semitones: Option<f32>,
    /// `keep_pitch`: whether a speed change keeps the pitch.
    #[serde(default)]
    pub keep: Option<bool>,
    #[serde(flatten)]
    pub guard: Guard,
}

fn ease(value: Option<&serde_json::Value>) -> Result<Ease, ToolError> {
    match value {
        None => Ok(Ease::Linear),
        Some(serde_json::Value::String(name)) => match name.as_str() {
            "linear" => Ok(Ease::Linear),
            "hold" => Ok(Ease::Hold),
            "smooth" => Ok(Ease::Bezier {
                x1: 0.42,
                y1: 0.0,
                x2: 0.58,
                y2: 1.0,
            }),
            other => Err(ToolError::invalid(format!(
                "ease is linear, smooth, hold or [x1, y1, x2, y2], not {other}"
            ))),
        },
        Some(serde_json::Value::Array(points)) => {
            let numbers: Option<Vec<f32>> = points
                .iter()
                .map(|p| p.as_f64().map(|n| n as f32))
                .collect();
            match numbers.as_deref() {
                Some([x1, y1, x2, y2]) => Ok(Ease::Bezier {
                    x1: *x1,
                    y1: *y1,
                    x2: *x2,
                    y2: *y2,
                }),
                _ => Err(ToolError::invalid("A bezier ease is four numbers")),
            }
        }
        Some(_) => Err(ToolError::invalid("ease is a name or four numbers")),
    }
}

fn set_params(effect: &mut Effect, params: &BTreeMap<String, f32>) -> Result<(), ToolError> {
    for (key, value) in params {
        let key: ParamKey = named("parameter key", key)?;
        if !look::spec(effect.kind).iter().any(|spec| spec.key == key) {
            return Err(ToolError::invalid(format!(
                "{} has no parameter {}",
                inspector::effect_title(effect.kind),
                name_of(&key)
            )));
        }
        effect.params.retain(|param| param.key != key);
        effect.params.push(ParamValue { key, value: *value });
    }
    Ok(())
}

fn change_effect(
    effect: &mut Effect,
    params: &EffectParams,
    project: &Project,
) -> Result<(), ToolError> {
    set_params(effect, &params.params)?;
    if let Some(mix) = params.mix {
        effect.mix = mix;
    }
    if let Some(enabled) = params.enabled {
        effect.enabled = enabled;
    }
    if let Some(asset) = &params.asset_id {
        effect.asset_id = Some(super::resolve_asset_id(project, asset)?);
    }
    if let Some(points) = &params.points {
        effect.points = points.clone();
    }
    Ok(())
}

/// The effect a SetEffects inside `command` added to `clip_id`.
fn added_effects<'a>(command: &'a mut Command, before: &Project) -> Vec<&'a mut Effect> {
    match command {
        Command::SetEffects { clip_id, effects } => {
            let old: Vec<Uuid> = before
                .clip(*clip_id)
                .map(|(_, clip)| clip.effects.iter().map(|e| e.id).collect())
                .unwrap_or_default();
            effects
                .iter_mut()
                .filter(|effect| !old.contains(&effect.id))
                .collect()
        }
        Command::Batch { commands, .. } => commands
            .iter_mut()
            .flat_map(|command| added_effects(command, before))
            .collect(),
        _ => Vec::new(),
    }
}

fn mask_of(spec: &MaskSpec, existing: &[Mask]) -> Result<Mask, ToolError> {
    let mut mask = match spec.mask_id {
        Some(id) => existing
            .iter()
            .find(|mask| mask.id == id)
            .cloned()
            .ok_or_else(|| ToolError::not_found(format!("No mask {id} on that clip")))?,
        None => Mask::new(spec.shape.unwrap_or(MaskShape::Rectangle)),
    };
    if let Some(shape) = spec.shape {
        mask.shape = shape;
    }
    if let Some(op) = spec.op {
        mask.op = op;
    }
    if let Some(invert) = spec.invert {
        mask.invert = invert;
    }
    if let Some(center) = spec.center {
        mask.center = center;
    }
    if let Some(size) = spec.size {
        mask.size = size;
    }
    if let Some(rotation) = spec.rotation {
        mask.rotation = rotation;
    }
    if let Some(feather) = spec.feather {
        mask.feather = feather;
    }
    if let Some(roundness) = spec.roundness {
        mask.roundness = roundness;
    }
    Ok(mask)
}

fn ramp(text: &str) -> Result<Ramp, ToolError> {
    let wanted = text.trim().to_lowercase().replace(['_', '-'], " ");
    Ramp::ALL
        .into_iter()
        .find(|ramp| ramp.label().to_lowercase() == wanted)
        .ok_or_else(|| {
            ToolError::invalid(
                "ramp is ramp_up, ramp_down, montage, hero, bullet, flash_in or flash_out",
            )
        })
}

#[tool_router(router = effect_tools, vis = "pub(crate)")]
impl NeptuneVideoServer {
    #[tool(
        description = "Everything effects, masks, transitions, looks, blend modes, keyframes and \
                       text animations can be: kinds, parameter keys with ranges and defaults, \
                       and which keys animate.",
        annotations(read_only_hint = true)
    )]
    async fn effects_catalog(
        &self,
    ) -> Result<Json<serde_json::Map<String, serde_json::Value>>, ToolError> {
        let mut out = serde_json::Map::new();
        out.insert(
            "effects".into(),
            EffectKind::ALL
                .iter()
                .map(|kind| {
                    let (_, family) = inspector::effect_family(*kind);
                    serde_json::json!({
                        "kind": kind,
                        "name": inspector::effect_title(*kind),
                        "family": family,
                        "audio": kind.is_audio(),
                        "needs_asset": kind.needs_asset(),
                        "curve": kind.is_curve(),
                        "offered_in_window": kind.is_audio() || crate::ui::browser::offered(*kind),
                        "params": specs(AnimTarget::Effect(*kind), look::spec(*kind)),
                    })
                })
                .collect(),
        );
        out.insert(
            "transitions".into(),
            transitions::KINDS
                .iter()
                .map(|kind| {
                    serde_json::json!({
                        "kind": kind,
                        "name": transitions::name(*kind),
                        "params": look::transition_spec(*kind).iter().map(|spec| serde_json::json!({"key": spec.key, "min": spec.min, "max": spec.max, "default": spec.default})).collect::<Vec<_>>(),
                    })
                })
                .collect(),
        );
        out.insert(
            "clip_params".into(),
            specs(AnimTarget::Clip, look::clip_spec()).into(),
        );
        out.insert(
            "mask_params".into(),
            specs(AnimTarget::Mask, look::mask_spec()).into(),
        );
        out.insert(
            "looks".into(),
            inspector::looks()
                .iter()
                .map(|(preset, name)| serde_json::json!({"look": preset, "name": name}))
                .collect(),
        );
        out.insert("blend_modes".into(), blend_modes());
        out.insert(
            "text_animations".into(),
            TextAnimationKind::ALL
                .iter()
                .map(|kind| serde_json::json!(kind))
                .collect(),
        );
        out.insert(
            "audio_presets".into(),
            serde_json::json!(["voice", "three_band_eq"]),
        );
        out.insert(
            "speed_ramps".into(),
            Ramp::ALL
                .iter()
                .map(|ramp| serde_json::json!(ramp.label().to_lowercase().replace(' ', "_")))
                .collect(),
        );
        out.insert(
            "limits".into(),
            serde_json::json!({
                "effects_per_clip": look::MAX_EFFECTS,
                "masks_per_clip": look::MAX_MASKS,
                "curve_points": look::MAX_CURVE_POINTS,
                "longest_transition_seconds": look::MAX_TRANSITION.as_seconds(),
            }),
        );
        Ok(Json(out))
    }

    #[tool(
        description = "Effects on clips: add (picture or audio effect, or an audio preset), \
                          update parameters, remove, set masks, set the blend mode, or apply a \
                          colour look. Parameter keys and ranges are in effects_catalog."
    )]
    async fn effect_edit(
        &self,
        Parameters(params): Parameters<EffectParams>,
    ) -> Result<Json<Edited>, ToolError> {
        let guard = params.guard;
        self.edit(guard, move |snapshot| {
            let project = &snapshot.project;
            if params.clip_ids.is_empty() {
                return Err(ToolError::invalid("Name the clips in clip_ids"));
            }
            for id in &params.clip_ids {
                clip(project, *id)?;
            }
            let one = || match params.clip_ids.as_slice() {
                [id] => Ok(*id),
                _ => Err(ToolError::invalid("This op takes one clip")),
            };
            match params.op.as_str() {
                "add" => {
                    if let Some(preset) = &params.preset {
                        let preset = match preset.as_str() {
                            "voice" => crate::audio::Preset::Voice,
                            "three_band_eq" => crate::audio::Preset::ThreeBandEq,
                            other => {
                                return Err(ToolError::invalid(format!("No audio preset {other}")));
                            }
                        };
                        return crate::audio::give_command(
                            project,
                            &params.clip_ids,
                            crate::audio::Given::Preset(preset),
                        )
                        .map_err(refused);
                    }
                    let kind: EffectKind = named(
                        "effect kind",
                        params
                            .kind
                            .as_deref()
                            .ok_or_else(|| ToolError::invalid("Give kind"))?,
                    )?;
                    let mut command = if kind.is_audio() {
                        crate::audio::give_command(
                            project,
                            &params.clip_ids,
                            crate::audio::Given::Kind(kind),
                        )
                        .map_err(refused)?
                    } else {
                        let mut commands = Vec::new();
                        for id in &params.clip_ids {
                            let (track, picture) = clip(project, *id)?;
                            if track.kind != TrackKind::Video
                                || !matches!(
                                    picture.content,
                                    editor_core::ClipContent::Media { .. }
                                )
                            {
                                return Err(refused(
                                    "Only a picture clip (video or image) takes picture effects",
                                ));
                            }
                            let mut effects = picture.effects.clone();
                            effects
                                .insert(inspector::place_effect(&effects, kind), Effect::new(kind));
                            commands.push(Command::SetEffects {
                                clip_id: *id,
                                effects,
                            });
                        }
                        batch("Add effect", commands)?
                    };
                    for effect in added_effects(&mut command, project) {
                        change_effect(effect, &params, project)?;
                    }
                    Ok(command)
                }
                "update" | "remove" => {
                    let (_, target) = clip(project, one()?)?;
                    let effect_id = params
                        .effect_id
                        .ok_or_else(|| ToolError::invalid("Give effect_id"))?;
                    let mut effects = target.effects.clone();
                    let index = effects
                        .iter()
                        .position(|effect| effect.id == effect_id)
                        .ok_or_else(|| {
                            ToolError::not_found(format!("No effect {effect_id} on that clip"))
                        })?;
                    if params.op == "remove" {
                        effects.remove(index);
                    } else {
                        change_effect(&mut effects[index], &params, project)?;
                    }
                    Ok(Command::SetEffects {
                        clip_id: target.id,
                        effects,
                    })
                }
                "masks" => {
                    let specs = params
                        .masks
                        .as_ref()
                        .ok_or_else(|| ToolError::invalid("Give masks"))?;
                    let commands = params
                        .clip_ids
                        .iter()
                        .map(|id| {
                            let (_, target) = clip(project, *id)?;
                            let masks = specs
                                .iter()
                                .map(|spec| mask_of(spec, &target.masks))
                                .collect::<Result<Vec<_>, _>>()?;
                            Ok(Command::SetMasks {
                                clip_id: *id,
                                masks,
                            })
                        })
                        .collect::<Result<Vec<_>, ToolError>>()?;
                    batch("Change masks", commands)
                }
                "blend" => {
                    let mode: BlendMode = named(
                        "blend mode",
                        params
                            .mode
                            .as_deref()
                            .ok_or_else(|| ToolError::invalid("Give mode"))?,
                    )?;
                    batch(
                        "Change blend mode",
                        params
                            .clip_ids
                            .iter()
                            .map(|id| Command::SetBlendMode { clip_id: *id, mode })
                            .collect(),
                    )
                }
                "look" => {
                    let name = params
                        .look
                        .as_deref()
                        .ok_or_else(|| ToolError::invalid("Give look"))?;
                    if name == "none" {
                        let commands = params
                            .clip_ids
                            .iter()
                            .map(|id| {
                                let (_, target) = clip(project, *id)?;
                                Ok(Command::SetEffects {
                                    clip_id: *id,
                                    effects: look::with_look(&target.effects, None),
                                })
                            })
                            .collect::<Result<Vec<_>, ToolError>>()?;
                        return batch("Remove look", commands);
                    }
                    let preset: LookPreset = named("look", name)?;
                    Ok(Command::ApplyPreset {
                        clip_ids: params.clip_ids.clone(),
                        preset,
                        strength: params.strength.unwrap_or(1.0),
                    })
                }
                other => Err(refused(format!(
                    "op is add, update, remove, masks, blend or look, not {other}"
                ))),
            }
        })
        .await
    }

    #[tool(
        description = "Set or remove the transition at a clip's start (in) or end (out). An \
                          out transition between two clips is the next clip's in transition, as \
                          in the window."
    )]
    async fn transition_set(
        &self,
        Parameters(params): Parameters<TransitionParams>,
    ) -> Result<Json<Edited>, ToolError> {
        self.edit(params.guard, move |snapshot| {
            let project = &snapshot.project;
            clip(project, params.clip_id)?;
            let (id, edge) =
                transitions::place(project, params.clip_id, params.edge).map_err(refused)?;
            if params.kind == "none" {
                let (_, there) = clip(project, id)?;
                if transitions::of(there, edge).is_none() {
                    return Err(refused("There is no transition there"));
                }
                return Ok(transitions::set_command(id, edge, None));
            }
            let kind: TransitionKind = named("transition kind", &params.kind)?;
            let (_, there) = clip(project, id)?;
            let mut transition = match transitions::of(there, edge) {
                Some(existing) => transitions::with_kind(existing, kind),
                None => transitions::default_of(&[], kind),
            };
            if let Some(duration) = &params.duration {
                transition.duration = duration.length(project.settings.frame_rate)?;
            }
            let fitted = transitions::fitted(project, there, transition.duration)
                .ok_or_else(|| refused("That clip is too short for a transition"))?;
            if params.duration.is_some() && fitted != transition.duration {
                return Err(refused(format!(
                    "That transition can be at most {:.3} s there",
                    fitted.as_seconds()
                )));
            }
            transition.duration = fitted;
            Ok(transitions::set_command(id, edge, Some(transition)))
        })
        .await
    }

    #[tool(
        description = "Keyframes: animate a clip property (position, scale, rotation, \
                          opacity, crop, volume) or an effect's or mask's parameter over time. \
                          Key times are timeline times inside the clip."
    )]
    async fn keyframes_set(
        &self,
        Parameters(params): Parameters<KeyParams>,
    ) -> Result<Json<Edited>, ToolError> {
        self.edit(params.guard, move |snapshot| {
            let project = &snapshot.project;
            let rate = project.settings.frame_rate;
            let (_, target) = clip(project, params.clip_id)?;
            let key: ParamKey = named("parameter key", &params.key)?;
            let anim_target = match params.target_id {
                None => AnimTarget::Clip,
                Some(id) => match (
                    target.effects.iter().find(|effect| effect.id == id),
                    target.masks.iter().any(|mask| mask.id == id),
                ) {
                    (Some(effect), _) => AnimTarget::Effect(effect.kind),
                    (None, true) => AnimTarget::Mask,
                    _ => {
                        return Err(ToolError::not_found(format!(
                            "No effect or mask {id} on that clip"
                        )));
                    }
                },
            };
            if !look::animatable(anim_target, key) {
                return Err(refused(format!("{} does not animate there", params.key)));
            }
            let mut keys = Vec::new();
            for spec in &params.keys {
                let at = spec.at.at(rate)?;
                if at < target.start || at > target.end() {
                    return Err(refused(format!(
                        "A keyframe at {:.3} s is outside the clip ({:.3} s to {:.3} s)",
                        at.as_seconds(),
                        target.start.as_seconds(),
                        target.end().as_seconds()
                    )));
                }
                keys.push(Keyframe {
                    at: Time(at.0 - target.start.0),
                    value: spec.value,
                    ease: ease(spec.ease.as_ref())?,
                });
            }
            let replace = match params.mode.as_deref() {
                None | Some("merge") => false,
                Some("replace") => true,
                Some(other) => {
                    return Err(ToolError::invalid(format!(
                        "mode is merge or replace, not {other}"
                    )));
                }
            };
            if !replace {
                let mut merged: Vec<Keyframe> = target
                    .animation
                    .iter()
                    .find(|track| track.target == params.target_id && track.key == key)
                    .map(|track| track.keys.clone())
                    .unwrap_or_default();
                for key in keys {
                    merged.retain(|old| old.at != key.at);
                    merged.push(key);
                }
                keys = merged;
            }
            keys.sort_by_key(|key| key.at);
            Ok(Command::SetKeyframes {
                clip_id: target.id,
                target: params.target_id,
                key,
                keys,
            })
        })
        .await
    }

    #[tool(
        description = "Change how a clip plays over time: a named speed ramp, reverse, a \
                          freeze frame, reset to normal speed, or its sound's pitch."
    )]
    async fn retime_set(
        &self,
        Parameters(params): Parameters<RetimeParams>,
    ) -> Result<Json<Edited>, ToolError> {
        self.edit(params.guard, move |snapshot| {
            let project = &snapshot.project;
            let rate = project.settings.frame_rate;
            let (track, target) = clip(project, params.clip_id)?;
            let timed = || {
                if speed::timed(project, track, target) {
                    Ok(())
                } else {
                    Err(refused("Only a video clip has a speed to change"))
                }
            };
            match params.op.as_str() {
                "ramp" => {
                    timed()?;
                    let ramp = ramp(
                        params
                            .ramp
                            .as_deref()
                            .ok_or_else(|| ToolError::invalid("Give ramp"))?,
                    )?;
                    let curve = ramp.over(&Curve::of(target));
                    speed::curve_command(project, target.id, &curve, ramp.label())
                        .ok_or_else(|| refused("That ramp does not fit the clip"))
                }
                "reverse" => {
                    timed()?;
                    let mut curve = Curve::of(target);
                    curve.reverse();
                    speed::curve_command(project, target.id, &curve, "Reverse clip")
                        .ok_or_else(|| refused("That clip cannot be played backwards"))
                }
                "reset" => {
                    timed()?;
                    speed::constant_command(project, target.id, 1.0)
                        .ok_or_else(|| refused("That clip already plays at normal speed"))
                }
                "freeze" => {
                    let at = params
                        .at
                        .as_ref()
                        .ok_or_else(|| ToolError::invalid("Give at"))?
                        .at(rate)?;
                    let hold = match &params.hold {
                        Some(hold) => hold.length(rate)?,
                        None => speed::HOLD,
                    };
                    speed::freeze_command(project, target.id, at, hold).map_err(refused)
                }
                "pitch" => {
                    let semitones = params
                        .semitones
                        .ok_or_else(|| ToolError::invalid("Give semitones"))?;
                    speed::pitch_command(project, target.id, semitones).map_err(refused)
                }
                "keep_pitch" => {
                    let keep = params.keep.ok_or_else(|| ToolError::invalid("Give keep"))?;
                    speed::keep_pitch_command(project, target.id, keep).map_err(refused)
                }
                other => Err(refused(format!(
                    "op is ramp, reverse, freeze, reset, pitch or keep_pitch, not {other}"
                ))),
            }
        })
        .await
    }
}
