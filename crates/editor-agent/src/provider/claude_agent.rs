//! Claude Code's print mode as a conversation: requests in and steps out as
//! stream JSON, one line each, with the Neptune Video tool server as its only tools.
use super::{Heard, summary};
use crate::AiError;
use serde_json::{Value, json};

/// The prefix Claude gives the tools of the server named `neptune-video`.
const PREFIX: &str = "mcp__neptune-video__";

#[derive(Default)]
pub(super) struct ClaudeAgent {
    model: Option<String>,
}

impl ClaudeAgent {
    pub(super) fn ask(&self, prompt: &str) -> Value {
        json!({"type":"user","message":{"role":"user","content":prompt}})
    }

    pub(super) fn receive(&mut self, frame: &Value) -> Result<Vec<Heard>, AiError> {
        let mut heard = Vec::new();
        match frame["type"].as_str() {
            Some("system") if frame["subtype"] == "init" => {
                let status = (frame["mcp_servers"].as_array().into_iter().flatten())
                    .find(|server| server["name"] == "neptune-video")
                    .and_then(|server| server["status"].as_str())
                    .unwrap_or("missing");
                if status != "connected" {
                    return Err(AiError::Provider(format!(
                        "Neptune Video's editing tools did not start for Claude ({status})"
                    )));
                }
                self.model = frame["model"]
                    .as_str()
                    .filter(|model| model.len() <= 128)
                    .map(str::to_owned);
            }
            Some("assistant") => {
                for block in frame["message"]["content"].as_array().into_iter().flatten() {
                    match block["type"].as_str() {
                        Some("text") => {
                            if let Some(text) = block["text"].as_str()
                                && !text.trim().is_empty()
                            {
                                heard.push(Heard::Said(text.trim().to_owned()));
                            }
                        }
                        Some("tool_use") => {
                            let name = block["name"].as_str().unwrap_or_default();
                            heard.push(Heard::Called {
                                call: block["id"].as_str().unwrap_or_default().to_owned(),
                                tool: name.strip_prefix(PREFIX).unwrap_or(name).to_owned(),
                                op: block["input"]["op"].as_str().map(str::to_owned),
                            });
                        }
                        _ => {}
                    }
                }
            }
            Some("user") => {
                for block in frame["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] != "tool_result" {
                        continue;
                    }
                    let failed = block["is_error"] == true;
                    let (refused, detail) = summary(&block["content"]);
                    heard.push(Heard::Returned {
                        call: block["tool_use_id"].as_str().unwrap_or_default().to_owned(),
                        ok: !failed && !refused,
                        detail,
                    });
                }
            }
            Some("result") => {
                if frame["subtype"] == "success" && frame["is_error"] != true {
                    let model = (frame["modelUsage"].as_object())
                        .and_then(|models| models.keys().next().cloned())
                        .filter(|model| model.len() <= 128)
                        .or_else(|| self.model.clone());
                    heard.push(Heard::Finished { model });
                } else if frame["subtype"] == "error_max_turns" {
                    return Err(AiError::Provider(
                        "Claude stopped after the most steps one conversation may take; start a new one".into(),
                    ));
                } else {
                    return Err(AiError::Provider(
                        "Claude could not complete the request; check its sign-in, model access and usage limits".into(),
                    ));
                }
            }
            _ => {}
        }
        Ok(heard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_of_one_request_becomes_its_steps() {
        let mut agent = ClaudeAgent::default();
        let lines = [
            json!({"type":"system","subtype":"init","model":"claude-opus-5-5",
                   "mcp_servers":[{"name":"neptune-video","status":"connected"}]}),
            json!({"type":"assistant","message":{"content":[
                {"type":"text","text":"Trimming the opening."},
                {"type":"tool_use","id":"t1","name":"mcp__neptune-video__clip_edit","input":{"op":"trim"}}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1",
                "content":[{"type":"text","text":"{\"label\":\"Trim clip\",\"changed\":true,\"revision\":4}"}]}]}}),
            json!({"type":"result","subtype":"success","is_error":false,"result":"Done.",
                   "modelUsage":{"claude-opus-5-5":{}}}),
        ];
        let heard: Vec<Heard> = lines
            .iter()
            .flat_map(|line| agent.receive(line).unwrap())
            .collect();
        assert_eq!(
            heard,
            [
                Heard::Said("Trimming the opening.".into()),
                Heard::Called {
                    call: "t1".into(),
                    tool: "clip_edit".into(),
                    op: Some("trim".into())
                },
                Heard::Returned {
                    call: "t1".into(),
                    ok: true,
                    detail: Some("Trim clip".into())
                },
                Heard::Finished {
                    model: Some("claude-opus-5-5".into())
                },
            ]
        );
    }

    #[test]
    fn tools_that_did_not_start_or_a_failed_request_end_the_conversation() {
        let mut agent = ClaudeAgent::default();
        assert!(
            agent
                .receive(&json!({"type":"system","subtype":"init",
                    "mcp_servers":[{"name":"neptune-video","status":"failed"}]}))
                .is_err()
        );
        assert!(
            agent
                .receive(
                    &json!({"type":"result","subtype":"error_during_execution","is_error":true})
                )
                .is_err()
        );
    }

    #[test]
    fn a_refusal_is_a_failed_step_with_its_reason() {
        let mut agent = ClaudeAgent::default();
        let heard = agent
            .receive(
                &json!({"type":"user","message":{"content":[{"type":"tool_result",
                "tool_use_id":"t2","is_error":true,
                "content":"{\"code\":\"stale_revision\",\"message\":\"The project changed\"}"}]}}),
            )
            .unwrap();
        assert_eq!(
            heard,
            [Heard::Returned {
                call: "t2".into(),
                ok: false,
                detail: Some("The project changed".into())
            }]
        );
    }
}
