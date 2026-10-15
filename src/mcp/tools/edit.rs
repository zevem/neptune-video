//! Core commands as they are: any edit the editor knows, in a batch that is
//! one history entry, checked before it is applied. New editor features are
//! reachable here as soon as they are commands.
use super::Guard;
use crate::mcp::{errors::ToolError, server::AsterServer, workspace::Edited};
use editor_core::Command;
use rmcp::{Json, handler::server::wrapper::Parameters, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

/// The most commands one call applies.
const MOST_COMMANDS: usize = 10_000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ApplyParams {
    /// Core commands (edit_schema describes them), applied in order as one
    /// undoable step: all of them or none. Times are integer microseconds.
    pub commands: Vec<serde_json::Value>,
    /// The history entry's name; the command's own for one command.
    #[serde(default)]
    pub label: Option<String>,
    #[serde(flatten)]
    pub guard: Guard,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SchemaParams {
    /// One command type (add_clip, set_effects, …) instead of the list.
    #[serde(default)]
    pub command: Option<String>,
}

fn command_names(schema: &serde_json::Value) -> Vec<String> {
    schema["oneOf"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|variant| {
            variant["properties"]["type"]["const"]
                .as_str()
                .map(str::to_owned)
        })
        .collect()
}

#[tool_router(router = edit_tools, vis = "pub(crate)")]
impl AsterServer {
    #[tool(
        description = "Apply core editing commands as one undoable, all-or-nothing step: \
                          every edit the editor has, including ones the typed tools do not \
                          cover. dry_run checks them first."
    )]
    async fn edit_apply(
        &self,
        Parameters(params): Parameters<ApplyParams>,
    ) -> Result<Json<Edited>, ToolError> {
        if params.commands.is_empty() || params.commands.len() > MOST_COMMANDS {
            return Err(ToolError::invalid(format!(
                "Give 1 to {MOST_COMMANDS} commands"
            )));
        }
        let mut commands = Vec::with_capacity(params.commands.len());
        for (index, value) in params.commands.into_iter().enumerate() {
            let command: Command = serde_json::from_value(value).map_err(|error| {
                ToolError::invalid(format!(
                    "Command {index} is not a command ({error}); edit_schema describes them"
                ))
            })?;
            commands.push(command);
        }
        let command = match (commands.len(), params.label) {
            (1, None) => commands.remove(0),
            (_, label) => Command::Batch {
                label: label.unwrap_or_else(|| "Agent edit".into()),
                commands,
            },
        };
        self.edit(params.guard, move |_| Ok(command)).await
    }

    #[tool(
        description = "The JSON schema of core commands for edit_apply: the list of command \
                       types, or one command's schema with the types it uses.",
        annotations(read_only_hint = true)
    )]
    async fn edit_schema(
        &self,
        Parameters(params): Parameters<SchemaParams>,
    ) -> Result<Json<serde_json::Map<String, serde_json::Value>>, ToolError> {
        let schema = serde_json::to_value(schemars::schema_for!(Command)).unwrap_or_default();
        let mut out = serde_json::Map::new();
        match params.command {
            None => {
                out.insert("commands".into(), command_names(&schema).into());
                out.insert(
                    "note".into(),
                    "Times are integer microseconds; keyframe and word times are from the \
                     clip's start. Ask for one command to see its fields."
                        .into(),
                );
            }
            Some(name) => {
                let variant = schema["oneOf"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|variant| variant["properties"]["type"]["const"] == name.as_str())
                    .cloned()
                    .ok_or_else(|| ToolError::not_found(format!("No command {name}")))?;
                out.insert("command".into(), variant);
                out.insert("$defs".into(), schema["$defs"].clone());
            }
        }
        Ok(Json(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_names_every_command() {
        let schema = serde_json::to_value(schemars::schema_for!(Command)).unwrap();
        let names = command_names(&schema);
        assert!(names.contains(&"add_clip".to_owned()), "{names:?}");
        assert!(names.contains(&"batch".to_owned()));
        assert!(names.len() > 40);
    }
}
