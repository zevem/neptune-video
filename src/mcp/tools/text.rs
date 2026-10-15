//! Titles, graphics and captions, and the templates the Text tab offers.
use super::{Guard, batch, clip, refused, stamp};
use crate::{
    mcp::{
        errors::{ToolError, codes},
        server::AsterServer,
        time::TimeSpec,
        tools::effects::named,
        workspace::Edited,
    },
    speech::CueLength,
    ui::{
        Action, captions_tab,
        text_tab::{self, Group, Look},
    },
};
use editor_core::{
    ClipContent, Command, ElementKind, Project, TextAnimation, TextAnimationKind, TextStyle,
    TextUnit, Time, TransitionEdge,
    captions::{SpokenWord, SubtitleFormat},
};
use rmcp::{
    Json,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    tool, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AnimationSpec {
    /// fade, pop, bounce, typewriter, slide_up, slide_down, slide_left,
    /// slide_right or wipe; `none` removes it.
    pub kind: String,
    /// whole (the default), line, word or letter.
    #[serde(default)]
    pub unit: Option<TextUnit>,
    /// Half a second by default.
    #[serde(default)]
    pub duration: Option<TimeSpec>,
    /// From the clip's start (entrance) or to its end (exit).
    #[serde(default)]
    pub delay: Option<TimeSpec>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TextAddParams {
    /// The words of a title. For a graphic template, the texts of its text
    /// elements in order go in `texts`.
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub texts: Vec<String>,
    /// A template from templates {op: list}; a plain centred title by
    /// default.
    #[serde(default)]
    pub template_id: Option<Uuid>,
    pub at: TimeSpec,
    /// Three seconds by default, or what the template's animations need.
    #[serde(default)]
    pub duration: Option<TimeSpec>,
    /// A text track; one with room is found or made by default.
    #[serde(default)]
    pub track_id: Option<Uuid>,
    #[serde(default)]
    pub font_size: Option<f32>,
    /// RGBA, 0 to 255 each.
    #[serde(default)]
    pub color: Option<[u8; 4]>,
    /// Canvas pixels from the canvas's centre, y down.
    #[serde(default)]
    pub position: Option<[f32; 2]>,
    /// Fields of a TextStyle to set (edit_schema has it): font {family,
    /// weight, italic}, align, line_height, tracking, wrap, case, outline,
    /// shadow, background, highlight. Sizes are fractions of the font size.
    #[serde(default)]
    pub style: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    pub enter: Option<AnimationSpec>,
    #[serde(default)]
    pub exit: Option<AnimationSpec>,
    #[serde(flatten)]
    pub guard: Guard,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TextEditParams {
    /// Titles or graphics to change.
    pub clip_ids: Vec<Uuid>,
    #[serde(default)]
    pub text: Option<String>,
    /// A graphic's text elements, in order.
    #[serde(default)]
    pub texts: Vec<String>,
    #[serde(default)]
    pub font_size: Option<f32>,
    #[serde(default)]
    pub color: Option<[u8; 4]>,
    #[serde(default)]
    pub style: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    pub enter: Option<AnimationSpec>,
    #[serde(default)]
    pub exit: Option<AnimationSpec>,
    /// Give the titles a template's look, keeping their words and times.
    #[serde(default)]
    pub template_id: Option<Uuid>,
    #[serde(flatten)]
    pub guard: Guard,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CaptionParams {
    /// `from_transcript` (captions from a finished transcribe job),
    /// `import` (an .srt or .vtt file), `export` (to an .srt or .vtt file),
    /// `new_track`, `add`, `retime`, `split`, `merge`, `retype`,
    /// `language` or `translate` (a copy of a track with the texts given).
    pub op: String,
    #[serde(default)]
    pub job_id: Option<Uuid>,
    /// A language tag: en, pt-BR…
    #[serde(default)]
    pub language: Option<String>,
    /// `from_transcript`: phrases (the default), lines or words.
    #[serde(default)]
    pub length: Option<String>,
    /// `import`, `export`: the file, inside the roots.
    #[serde(default)]
    pub path: Option<String>,
    /// `export`: replace a file that is there.
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default)]
    pub track_id: Option<Uuid>,
    #[serde(default)]
    pub clip_id: Option<Uuid>,
    /// `add`, `split`, `retime`: a timeline time.
    #[serde(default)]
    pub at: Option<TimeSpec>,
    /// `retime`: start or end.
    #[serde(default)]
    pub edge: Option<String>,
    /// `retype`: the cue's new text.
    #[serde(default)]
    pub text: Option<String>,
    /// `translate`: one text for each cue of the track, in order.
    #[serde(default)]
    pub texts: Vec<String>,
    #[serde(flatten)]
    pub guard: Guard,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TemplateParams {
    /// `list`, `preview` (a picture of one) or `apply` (restyle titles).
    /// Add one with text_add {template_id}.
    pub op: String,
    #[serde(default)]
    pub template_id: Option<Uuid>,
    /// `list`: titles, captions, lower_thirds, title_cards, callouts,
    /// stickers, emoji or yours.
    #[serde(default)]
    pub group: Option<String>,
    /// `list`: words in the name.
    #[serde(default)]
    pub query: Option<String>,
    /// `apply`: the titles to restyle.
    #[serde(default)]
    pub clip_ids: Vec<Uuid>,
    #[serde(flatten)]
    pub guard: Guard,
}

fn group_slug(group: Group) -> &'static str {
    match group {
        Group::Yours => "yours",
        Group::Titles => "titles",
        Group::Captions => "captions",
        Group::LowerThirds => "lower_thirds",
        Group::TitleCards => "title_cards",
        Group::Callouts => "callouts",
        Group::Stickers => "stickers",
        Group::Emoji => "emoji",
    }
}

/// Every template: the built-in ones and the person's own.
pub fn templates(presets: Option<&std::path::Path>) -> Vec<Arc<Look>> {
    let mut all: Vec<Arc<Look>> = text_tab::builtin().to_vec();
    if let Some(path) = presets
        && let crate::presets::Read::Presets(presets) = crate::presets::Presets::read(path)
    {
        all.extend(
            presets
                .looks
                .iter()
                .map(|saved| Arc::new(Look::saved(saved))),
        );
    }
    all
}

fn template(presets: Option<&std::path::Path>, id: Uuid) -> Result<Arc<Look>, ToolError> {
    templates(presets)
        .into_iter()
        .find(|look| look.id == id)
        .ok_or_else(|| ToolError::not_found(format!("No template {id}; templates {{op: list}}")))
}

fn texts_of(content: &ClipContent) -> Vec<String> {
    match content {
        ClipContent::Text { text, .. } => vec![text.clone()],
        ClipContent::Graphic { elements, .. } => elements
            .iter()
            .filter_map(|element| match &element.kind {
                ElementKind::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect(),
        ClipContent::Media { .. } => Vec::new(),
    }
}

fn styled(
    style: &TextStyle,
    fields: &serde_json::Map<String, serde_json::Value>,
) -> Result<TextStyle, ToolError> {
    let mut value = serde_json::to_value(style).unwrap_or_default();
    if let Some(object) = value.as_object_mut() {
        for (key, field) in fields {
            object.insert(key.clone(), field.clone());
        }
    }
    let styled: TextStyle = serde_json::from_value(value)
        .map_err(|error| ToolError::invalid(format!("That style is not valid: {error}")))?;
    if !styled.validate() {
        return Err(ToolError::invalid(
            "That style is out of range. Sizes are fractions of the font size: line_height \
             0.5..4, tracking -0.5..2 em, outline width 0.005..0.5, shadow offset -2..2 each \
             and blur 0..1, background padding and radius 0..4; wrap is 0..16384 canvas \
             pixels; highlight scale 1..2 and upcoming 0..1.",
        ));
    }
    Ok(styled)
}

fn animation(
    spec: &AnimationSpec,
    rate: editor_core::FrameRate,
) -> Result<Option<TextAnimation>, ToolError> {
    if spec.kind == "none" {
        return Ok(None);
    }
    let kind: TextAnimationKind = named("text animation", &spec.kind)?;
    Ok(Some(TextAnimation {
        kind,
        unit: spec.unit.unwrap_or_else(|| kind.usual_unit()),
        duration: match &spec.duration {
            Some(duration) => duration.length(rate)?,
            None => Time::from_seconds(0.5),
        },
        delay: match &spec.delay {
            Some(delay) => delay.at(rate)?,
            None => Time::ZERO,
        },
    }))
}

/// What text_add and text_edit change on a title or graphic.
#[derive(Clone, Copy)]
struct TextChange<'a> {
    text: Option<&'a str>,
    texts: &'a [String],
    font_size: Option<f32>,
    color: Option<[u8; 4]>,
    style: Option<&'a serde_json::Map<String, serde_json::Value>>,
    enter: Option<&'a AnimationSpec>,
    exit: Option<&'a AnimationSpec>,
}

/// The edits that set a title's or graphic's words, size, colour, style and
/// animations.
fn text_commands(
    project: &Project,
    clip: &editor_core::Clip,
    change: &TextChange,
) -> Result<Vec<Command>, ToolError> {
    let TextChange {
        text,
        texts,
        font_size,
        color,
        style,
        enter,
        exit,
    } = *change;
    let rate = project.settings.frame_rate;
    let mut commands = Vec::new();
    match &clip.content {
        ClipContent::Text {
            text: had,
            font_size: had_size,
            color: had_color,
            style: had_style,
            ..
        } => {
            let new_text = text.or(texts.first().map(String::as_str));
            if new_text.is_some() || font_size.is_some() || color.is_some() {
                commands.push(Command::SetText {
                    clip_id: clip.id,
                    text: new_text.unwrap_or(had).to_owned(),
                    font_size: font_size.unwrap_or(*had_size),
                    color: color.unwrap_or(*had_color),
                });
            }
            if let Some(fields) = style {
                commands.push(Command::SetTextStyle {
                    clip_id: clip.id,
                    style: styled(had_style, fields)?,
                });
            }
            for (edge, spec) in [(TransitionEdge::In, enter), (TransitionEdge::Out, exit)] {
                if let Some(spec) = spec {
                    commands.push(Command::SetTextAnimation {
                        clip_id: clip.id,
                        edge,
                        animation: animation(spec, rate)?,
                    });
                }
            }
        }
        ClipContent::Graphic { size, elements } => {
            let mut words: Vec<String> = texts.to_vec();
            if let Some(text) = text
                && words.is_empty()
            {
                words.push(text.to_owned());
            }
            if words.is_empty() && font_size.is_none() && color.is_none() && style.is_none() {
                if enter.is_some() || exit.is_some() {
                    return Err(refused(
                        "A graphic's elements carry their own animations; edit them with edit_apply SetGraphic",
                    ));
                }
                return Ok(commands);
            }
            let mut elements = elements.clone();
            let mut given = words.into_iter();
            for element in &mut elements {
                if let ElementKind::Text {
                    text,
                    font_size: size,
                    color: tint,
                    style: had_style,
                } = &mut element.kind
                {
                    if let Some(word) = given.next() {
                        *text = word;
                    }
                    if let Some(new) = font_size {
                        *size = new;
                    }
                    if let Some(new) = color {
                        *tint = new;
                    }
                    if let Some(fields) = style {
                        *had_style = styled(had_style, fields)?;
                    }
                }
            }
            if given.next().is_some() {
                return Err(refused(format!(
                    "That graphic has {} text element(s)",
                    texts_of(&clip.content).len()
                )));
            }
            commands.push(Command::SetGraphic {
                clip_id: clip.id,
                size: *size,
                elements,
            });
        }
        ClipContent::Media { .. } => {
            return Err(refused("That clip is media, not a title or graphic"));
        }
    }
    Ok(commands)
}

fn length(text: Option<&str>) -> Result<CueLength, ToolError> {
    match text {
        None | Some("phrases") => Ok(CueLength::Phrases),
        Some("lines") => Ok(CueLength::Lines),
        Some("words") => Ok(CueLength::Words),
        Some(other) => Err(ToolError::invalid(format!(
            "length is phrases, lines or words, not {other}"
        ))),
    }
}

fn subtitle_format(path: &std::path::Path) -> Result<SubtitleFormat, ToolError> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .and_then(SubtitleFormat::from_extension)
        .ok_or_else(|| ToolError::invalid("A captions file ends in .srt or .vtt"))
}

#[tool_router(router = text_tools, vis = "pub(crate)")]
impl AsterServer {
    #[tool(
        description = "Add a title or graphic: plain text, or a template (lower third, \
                          title card, callout, sticker, emoji) with its texts, size, colour, \
                          style, position and entrance/exit animations."
    )]
    async fn text_add(
        &self,
        Parameters(params): Parameters<TextAddParams>,
    ) -> Result<Json<Edited>, ToolError> {
        let look = match params.template_id {
            Some(id) => template(self.shared.presets.as_deref(), id)?,
            None => Arc::new(Look::typed().clone()),
        };
        self.edit(params.guard, move |snapshot| {
            let project = &snapshot.project;
            let rate = project.settings.frame_rate;
            let at = params.at.at(rate)?;
            if let Some(track_id) = params.track_id {
                let track = super::track(project, track_id)?;
                if track.kind != editor_core::TrackKind::Text || track.locked {
                    return Err(refused("Titles go on an unlocked text track"));
                }
            }
            let duration = params
                .duration
                .as_ref()
                .map(|duration| duration.length(rate))
                .transpose()?;
            let (mut command, clip_id) =
                text_tab::adding_for(project, &look, params.track_id, at, duration);
            let Command::Batch { commands, .. } = &mut command else {
                return Err(ToolError::unavailable("No title was made"));
            };
            let added = commands
                .iter_mut()
                .find_map(|command| match command {
                    Command::AddClip { clip, .. } if clip.id == clip_id => Some(clip),
                    _ => None,
                })
                .ok_or_else(|| ToolError::unavailable("No title was made"))?;
            if let Some(position) = params.position {
                added.transform.position = position;
            }
            // The edits that set its words and look run on the clip as
            // added, in the same step.
            let extra = text_commands(
                project,
                added,
                &TextChange {
                    text: params.text.as_deref(),
                    texts: &params.texts,
                    font_size: params.font_size,
                    color: params.color,
                    style: params.style.as_ref(),
                    enter: params.enter.as_ref(),
                    exit: params.exit.as_ref(),
                },
            )?;
            commands.extend(extra);
            Ok(command)
        })
        .await
    }

    #[tool(
        description = "Change titles or graphics: words, size, colour, style fields, \
                          entrance and exit animations, or a template's look."
    )]
    async fn text_edit(
        &self,
        Parameters(params): Parameters<TextEditParams>,
    ) -> Result<Json<Edited>, ToolError> {
        let look = match params.template_id {
            Some(id) => Some(template(self.shared.presets.as_deref(), id)?),
            None => None,
        };
        self.edit(params.guard, move |snapshot| {
            let project = &snapshot.project;
            if params.clip_ids.is_empty() {
                return Err(ToolError::invalid("Name the clips in clip_ids"));
            }
            let mut commands = Vec::new();
            let more = params.text.is_some()
                || !params.texts.is_empty()
                || params.font_size.is_some()
                || params.color.is_some()
                || params.style.is_some()
                || params.enter.is_some()
                || params.exit.is_some();
            // The changes asked for besides a template's look are made on
            // the clips as the look leaves them, so they keep it.
            let mut styled_project = None;
            if let Some(look) = &look {
                match text_tab::restyle(project, look, &params.clip_ids) {
                    Action::Edit(command) => {
                        let after = editor_core::PreparedEdit::from_snapshot(
                            project.clone(),
                            snapshot.context_id,
                            snapshot.revision,
                            command.clone(),
                        )
                        .map_err(|error| ToolError::new("invalid_edit", error.to_string()))?
                        .after();
                        match command {
                            Command::Batch {
                                commands: styled, ..
                            } => commands.extend(styled),
                            command => commands.push(command),
                        }
                        styled_project = Some(after);
                    }
                    // They look like it already: only the other changes.
                    Action::Notify(_) if more => {}
                    Action::Notify(reason) => return Err(refused(reason)),
                    _ => return Err(ToolError::unavailable("The template gave no edit")),
                }
            }
            let base = styled_project.as_deref().unwrap_or(project);
            for id in &params.clip_ids {
                let (_, target) = clip(base, *id)?;
                commands.extend(text_commands(
                    project,
                    target,
                    &TextChange {
                        text: params.text.as_deref(),
                        texts: &params.texts,
                        font_size: params.font_size,
                        color: params.color,
                        style: params.style.as_ref(),
                        enter: params.enter.as_ref(),
                        exit: params.exit.as_ref(),
                    },
                )?);
            }
            batch("Edit text", commands)
        })
        .await
    }

    #[tool(
        description = "Captions: make them from a transcribe job's words, import or export \
                          .srt/.vtt, add, retime, split, merge or retype cues, set a track's \
                          language, or translate a track (you give the translated texts)."
    )]
    async fn captions(
        &self,
        Parameters(params): Parameters<CaptionParams>,
    ) -> Result<Json<serde_json::Map<String, serde_json::Value>>, ToolError> {
        let mut out = serde_json::Map::new();
        match params.op.as_str() {
            "export" => {
                let given = params
                    .path
                    .as_deref()
                    .ok_or_else(|| ToolError::invalid("Give path"))?;
                let path = self
                    .roots()
                    .destination(std::path::Path::new(given), params.overwrite)?;
                let format = subtitle_format(&path)?;
                let snapshot = self.on(|workspace| workspace.snapshot()).await?;
                // The project's one caption track unless another is named.
                let track_id = match params.track_id {
                    Some(id) => id,
                    None => {
                        let tracks: Vec<_> = editor_core::export::caption_tracks(&snapshot.project)
                            .map(|track| track.id)
                            .collect();
                        match tracks.as_slice() {
                            [one] => *one,
                            [] => return Err(ToolError::not_found("The project has no captions")),
                            _ => {
                                return Err(ToolError::invalid(
                                    "The project has more than one caption track; give track_id",
                                ));
                            }
                        }
                    }
                };
                let (text, count) =
                    captions_tab::exported(&snapshot.project, track_id, format).map_err(refused)?;
                let written = path.clone();
                tokio::task::spawn_blocking(move || captions_tab::write_file(&written, &text))
                    .await
                    .map_err(|_| ToolError::unavailable("The file was not written"))?
                    .map_err(|error| ToolError::new(codes::STORAGE, error))?;
                out.insert("path".into(), path.display().to_string().into());
                out.insert("cues".into(), count.into());
                return Ok(Json(out));
            }
            "import" => {
                let given = params
                    .path
                    .as_deref()
                    .ok_or_else(|| ToolError::invalid("Give path"))?;
                let path = self.roots().file(std::path::Path::new(given))?;
                subtitle_format(&path)?;
                let (cues, heard) =
                    tokio::task::spawn_blocking(move || captions_tab::read_file(&path))
                        .await
                        .map_err(|_| ToolError::unavailable("The file was not read"))?
                        .map_err(ToolError::invalid)?;
                let language = params
                    .language
                    .clone()
                    .or(heard)
                    .unwrap_or_else(|| "en".into());
                let edit = self
                    .edit(params.guard, move |snapshot| {
                        let (command, _, _) =
                            captions_tab::imported(&snapshot.project, &cues, &language)
                                .map_err(refused)?;
                        Ok(command)
                    })
                    .await?;
                out.insert(
                    "edit".into(),
                    serde_json::to_value(edit.0).unwrap_or_default(),
                );
                return Ok(Json(out));
            }
            "from_transcript" => {
                let job = params
                    .job_id
                    .ok_or_else(|| ToolError::invalid("Give job_id"))?;
                let result = self.shared.jobs.result(job, "transcribe")?;
                let words: Vec<SpokenWord> = result["words"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|word| {
                        Some(SpokenWord {
                            text: word["text"].as_str()?.to_owned(),
                            at: Time(word["at"]["us"].as_i64()?),
                            end: Time(word["end"]["us"].as_i64()?),
                        })
                    })
                    .collect();
                let language = params
                    .language
                    .clone()
                    .or_else(|| result["language"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| "en".into());
                let length = length(params.length.as_deref())?;
                let heard_in = (
                    result["project_id"].as_str().map(str::to_owned),
                    result["context_id"].as_str().map(str::to_owned),
                    result["revision"].as_u64(),
                );
                let explicit = params.guard.expected_revision.is_some();
                let edit = self
                    .edit(params.guard, move |snapshot| {
                        // The words are timed on the timeline as it was heard.
                        if heard_in.0.as_deref() != Some(snapshot.project.id.to_string().as_str()) {
                            return Err(ToolError::new(
                                codes::PROJECT_CHANGED,
                                "That transcript is of another project; transcribe this one",
                            ));
                        }
                        let same = heard_in.1.as_deref()
                            == Some(snapshot.context_id.to_string().as_str())
                            && heard_in.2 == Some(snapshot.revision);
                        if !same && !explicit {
                            return Err(ToolError::new(
                                "stale_revision",
                                format!(
                                    "The project changed since it was transcribed (revision {}, \
                                     now {}), so the words may no longer fall where they are \
                                     said. Transcribe again, or pass expected_revision: {} to \
                                     use them anyway.",
                                    heard_in.2.unwrap_or_default(),
                                    snapshot.revision,
                                    snapshot.revision
                                ),
                            )
                            .at(snapshot.revision));
                        }
                        let (command, _, _) =
                            crate::speech::generated(&snapshot.project, &words, &language, length)
                                .map_err(refused)?;
                        Ok(command)
                    })
                    .await?;
                out.insert(
                    "edit".into(),
                    serde_json::to_value(edit.0).unwrap_or_default(),
                );
                return Ok(Json(out));
            }
            _ => {}
        }
        let op = params.op.clone();
        let edit = self
            .edit(params.guard, move |snapshot| {
                let project = &snapshot.project;
                let rate = project.settings.frame_rate;
                let at = || {
                    params
                        .at
                        .as_ref()
                        .ok_or_else(|| ToolError::invalid("Give at"))?
                        .at(rate)
                };
                let clip_id = || {
                    params
                        .clip_id
                        .ok_or_else(|| ToolError::invalid("Give clip_id"))
                };
                let track_id = || {
                    params
                        .track_id
                        .ok_or_else(|| ToolError::invalid("Give track_id"))
                };
                let language = || {
                    let language = params
                        .language
                        .as_deref()
                        .ok_or_else(|| ToolError::invalid("Give language"))?;
                    if !editor_core::title::language_valid(language) {
                        return Err(ToolError::invalid(format!(
                            "{language} is not a language tag"
                        )));
                    }
                    Ok(language.to_owned())
                };
                match op.as_str() {
                    "new_track" => Ok(captions_tab::new_track(project, &language()?).0),
                    "add" => captions_tab::added(project, track_id()?, at()?)
                        .map(|(c, _)| c)
                        .map_err(refused),
                    "retime" => {
                        let edge = match params.edge.as_deref() {
                            Some("start") => crate::rules::Edge::Start,
                            Some("end") => crate::rules::Edge::End,
                            _ => return Err(ToolError::invalid("edge is start or end")),
                        };
                        captions_tab::retimed(project, clip_id()?, edge, at()?).map_err(refused)
                    }
                    "split" => captions_tab::split(project, clip_id()?, at()?).map_err(refused),
                    "merge" => captions_tab::merged(project, clip_id()?).map_err(refused),
                    "retype" => {
                        let text = params
                            .text
                            .as_deref()
                            .ok_or_else(|| ToolError::invalid("Give text"))?;
                        captions_tab::retyped(project, clip_id()?, text)
                            .map_err(refused)?
                            .ok_or_else(|| refused("The cue already says that"))
                    }
                    "language" => captions_tab::relabel(project, track_id()?, &language()?)
                        .ok_or_else(|| refused("The track already has that language")),
                    "translate" => {
                        let from = super::track(project, track_id()?)?;
                        let language = language()?;
                        Ok(Command::CopyCaptions {
                            from_track_id: from.id,
                            track_id: Uuid::new_v4(),
                            name: captions_tab::free_track_name(project, &language, None),
                            language,
                            texts: params.texts.clone(),
                        })
                    }
                    other => Err(refused(format!(
                        "op is from_transcript, import, export, new_track, add, retime, split, \
                         merge, retype, language or translate, not {other}"
                    ))),
                }
            })
            .await?;
        out.insert(
            "edit".into(),
            serde_json::to_value(edit.0).unwrap_or_default(),
        );
        Ok(Json(out))
    }

    #[tool(
        description = "Title and graphic templates: list them, preview one as a picture, \
                          or apply one's look to titles on the timeline. Add one with text_add \
                          {template_id}."
    )]
    async fn templates(
        &self,
        Parameters(params): Parameters<TemplateParams>,
    ) -> Result<CallToolResult, ToolError> {
        let presets = self.shared.presets.clone();
        match params.op.as_str() {
            "list" => {
                let query = params.query.as_deref().unwrap_or_default().to_lowercase();
                let list: Vec<serde_json::Value> = templates(presets.as_deref())
                    .iter()
                    .filter(|look| {
                        params
                            .group
                            .as_deref()
                            .is_none_or(|g| g == group_slug(look.group))
                    })
                    .filter(|look| query.is_empty() || look.name.to_lowercase().contains(&query))
                    .map(|look| {
                        serde_json::json!({
                            "template_id": look.id,
                            "name": look.name,
                            "group": group_slug(look.group),
                            "kind": if look.is_title() { "title" } else { "graphic" },
                            "texts": texts_of(&look.content),
                        })
                    })
                    .collect();
                Ok(CallToolResult::structured(
                    serde_json::json!({ "templates": list }),
                ))
            }
            "preview" => {
                let id = params
                    .template_id
                    .ok_or_else(|| ToolError::invalid("Give template_id"))?;
                let look = template(presets.as_deref(), id)?;
                let settings = match self.on(|workspace| workspace.snapshot()).await {
                    Ok(snapshot) => snapshot.project.settings.clone(),
                    Err(_) => editor_core::CanvasSettings::default(),
                };
                let mut project = Project::new("Template preview");
                project.settings = settings;
                let (command, clip_id) = text_tab::adding(&project, &look, None, Time::ZERO);
                let mut editor = editor_core::Editor::new(project)
                    .map_err(|error| ToolError::unavailable(error.to_string()))?;
                editor
                    .dispatch(command)
                    .map_err(|error| ToolError::unavailable(error.to_string()))?;
                let shown = editor.snapshot();
                let at = shown.clip(clip_id).map_or(Time::ZERO, |(_, clip)| {
                    Time(clip.start.0 + clip.duration.0 / 2)
                });
                let drawn = self
                    .shared
                    .frames
                    .draw(shown.clone(), vec![at], 960)
                    .await?;
                let picture = drawn
                    .first()
                    .ok_or_else(|| ToolError::unavailable("Nothing was drawn"))?;
                let png = crate::mcp::frames::png(picture)?;
                let mut result = CallToolResult::structured(serde_json::json!({
                    "template_id": look.id,
                    "name": look.name,
                    "at": stamp(at, &shown),
                    "width": picture.width,
                    "height": picture.height,
                }));
                result.content.push(ContentBlock::image(
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, png),
                    "image/png",
                ));
                Ok(result)
            }
            "apply" => {
                let id = params
                    .template_id
                    .ok_or_else(|| ToolError::invalid("Give template_id"))?;
                let look = template(presets.as_deref(), id)?;
                let clips = params.clip_ids.clone();
                let edit = self
                    .edit(params.guard, move |snapshot| {
                        let project = &snapshot.project;
                        let mut chosen = Vec::new();
                        for id in &clips {
                            chosen.extend(text_tab::restyled_with(project, *id));
                        }
                        match text_tab::restyle(project, &look, &chosen) {
                            Action::Edit(command) => Ok(command),
                            Action::Notify(reason) => Err(refused(reason)),
                            _ => Err(ToolError::unavailable("The template gave no edit")),
                        }
                    })
                    .await?;
                Ok(CallToolResult::structured(
                    serde_json::to_value(edit.0).unwrap_or_default(),
                ))
            }
            other => Err(ToolError::invalid(format!(
                "op is list, preview or apply, not {other}"
            ))),
        }
    }
}
