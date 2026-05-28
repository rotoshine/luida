//! Agent 실행 추상화 — AgentRuntime trait + stream 파서 + mock.
//!
//! 실제 CLI(claude/codex) spawn 구현은 `luida-runtimes` crate.
//! 여기서는 인터페이스 + 순수 파서 + 테스트용 ScriptedRuntime.

use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// worker 실행 입력.
#[derive(Debug, Clone, Default)]
pub struct AgentInvocation {
    pub prompt: String,
    pub cwd: Option<PathBuf>,
    pub session_id: Option<String>,
    pub system_context: Option<String>,
}

/// worker stream 이벤트 (stream-json 단순화 + escalation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    System { subtype: String },
    ToolUse { name: String },
    Text { text: String },
    Escalation { category: String, message: String },
    Result { success: bool, summary: Option<String> },
    Error { message: String },
}

/// 실행 결과 요약.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentOutcome {
    pub success: bool,
    pub saw_result: bool,
    pub summary: Option<String>,
    pub escalation: Option<(String, String)>,
}

/// Agent 런타임 — 콜백 기반 스트리밍 (async trait 회피).
pub trait AgentRuntime {
    /// model로 worker를 실행. 각 이벤트마다 on_event 호출. 종료 시 outcome.
    fn run(
        &self,
        model: &str,
        inv: &AgentInvocation,
        on_event: &mut dyn FnMut(&AgentEvent),
    ) -> Result<AgentOutcome>;
}

/// 이벤트 시퀀스로 outcome을 누적 갱신 (런타임 구현 공통 헬퍼).
pub fn fold_outcome(outcome: &mut AgentOutcome, ev: &AgentEvent) {
    match ev {
        AgentEvent::Result { success, summary } => {
            outcome.saw_result = true;
            outcome.success = *success;
            outcome.summary = summary.clone();
        }
        AgentEvent::Escalation { category, message } => {
            outcome.escalation = Some((category.clone(), message.clone()));
        }
        AgentEvent::Error { message } => {
            outcome.summary.get_or_insert_with(|| message.clone());
        }
        _ => {}
    }
}

/// result 이벤트가 없었으면 실패로 마감.
pub fn finalize_outcome(mut outcome: AgentOutcome, exit_ok: bool) -> AgentOutcome {
    if !outcome.saw_result {
        outcome.success = false;
        if outcome.summary.is_none() {
            outcome.summary = Some(if exit_ok {
                "worker가 result 이벤트 없이 종료".to_string()
            } else {
                "worker 비정상 종료 (result 없음)".to_string()
            });
        }
    }
    outcome
}

/// claude `--output-format stream-json` 한 줄을 파싱.
pub fn parse_claude_stream_line(line: &str) -> Option<AgentEvent> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let t = v.get("type")?.as_str()?;
    match t {
        "text" => {
            let text = v.get("text")?.as_str()?.to_string();
            // escalation 마커가 들어있으면 Escalation 우선
            if let Some((cat, msg)) = detect_escalation(&text) {
                return Some(AgentEvent::Escalation {
                    category: cat,
                    message: msg,
                });
            }
            Some(AgentEvent::Text { text })
        }
        "tool_use" => Some(AgentEvent::ToolUse {
            name: v.get("name")?.as_str()?.to_string(),
        }),
        "result" => {
            let is_error = v.get("is_error").and_then(|x| x.as_bool()).unwrap_or(false)
                || v.get("subtype").and_then(|x| x.as_str()) == Some("error")
                || v.get("success").and_then(|x| x.as_bool()) == Some(false);
            let summary = v
                .get("summary")
                .or_else(|| v.get("result"))
                .and_then(|x| x.as_str())
                .map(|s| s.to_string());
            Some(AgentEvent::Result {
                success: !is_error,
                summary,
            })
        }
        "system" => Some(AgentEvent::System {
            subtype: v.get("subtype")?.as_str()?.to_string(),
        }),
        "error" => Some(AgentEvent::Error {
            message: v.get("message")?.as_str()?.to_string(),
        }),
        _ => None,
    }
}

/// 알려진 escalation 카테고리.
pub const ESCALATION_CATEGORIES: &[&str] =
    &["system_error", "ambiguous_spec", "design_mismatch", "dangerous_op"];

/// `<<LUIDA_ASK category=...>>...<<END>>` 마커 감지.
/// **완성된 마커**(`<<END>>` 포함)만 인식 — 잘리거나 분할된 마커는 None (review M5).
pub fn detect_escalation(text: &str) -> Option<(String, String)> {
    let start = text.find("<<LUIDA_ASK")?;
    let after = &text[start..];
    // <<END>>가 없으면 불완전 마커 → 무시 (텍스트 끝까지 흡수 방지)
    let end_rel = after.find("<<END>>")?;
    let cat_marker = "category=";
    let cat_start = after.find(cat_marker)? + cat_marker.len();
    let body_start = after.find(">>")? + 2;
    // category= 가 헤더(`>>` 이전)에 있어야 정상 마커
    if cat_start >= body_start || body_start > end_rel {
        return None;
    }
    let cat_rest = &after[cat_start..];
    let cat_end = cat_rest.find([' ', '>'])?;
    let category = cat_rest[..cat_end].trim().to_string();
    if category.is_empty() {
        return None;
    }
    let message = after[body_start..end_rel].trim().to_string();
    Some((category, message))
}

/// 테스트용 — 미리 정의된 이벤트를 흘려보내는 런타임.
pub struct ScriptedRuntime {
    pub script: Vec<AgentEvent>,
    pub exit_ok: bool,
}

impl ScriptedRuntime {
    pub fn new(script: Vec<AgentEvent>) -> Self {
        Self {
            script,
            exit_ok: true,
        }
    }
}

impl AgentRuntime for ScriptedRuntime {
    fn run(
        &self,
        _model: &str,
        _inv: &AgentInvocation,
        on_event: &mut dyn FnMut(&AgentEvent),
    ) -> Result<AgentOutcome> {
        let mut outcome = AgentOutcome::default();
        for ev in &self.script {
            fold_outcome(&mut outcome, ev);
            on_event(ev);
        }
        Ok(finalize_outcome(outcome, self.exit_ok))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_text() {
        let ev = parse_claude_stream_line(r#"{"type":"text","text":"hi"}"#).unwrap();
        assert_eq!(ev, AgentEvent::Text { text: "hi".into() });
    }

    #[test]
    fn parse_tool_use() {
        let ev = parse_claude_stream_line(r#"{"type":"tool_use","name":"Edit"}"#).unwrap();
        assert_eq!(ev, AgentEvent::ToolUse { name: "Edit".into() });
    }

    #[test]
    fn parse_result_success_default() {
        let ev = parse_claude_stream_line(r#"{"type":"result"}"#).unwrap();
        assert_eq!(
            ev,
            AgentEvent::Result {
                success: true,
                summary: None
            }
        );
    }

    #[test]
    fn parse_result_is_error() {
        let ev = parse_claude_stream_line(r#"{"type":"result","is_error":true}"#).unwrap();
        assert_eq!(
            ev,
            AgentEvent::Result {
                success: false,
                summary: None
            }
        );
    }

    #[test]
    fn parse_result_summary_or_result_field() {
        let a = parse_claude_stream_line(r#"{"type":"result","summary":"done"}"#).unwrap();
        let b = parse_claude_stream_line(r#"{"type":"result","result":"fin"}"#).unwrap();
        assert!(matches!(a, AgentEvent::Result { summary: Some(s), .. } if s == "done"));
        assert!(matches!(b, AgentEvent::Result { summary: Some(s), .. } if s == "fin"));
    }

    #[test]
    fn parse_invalid_and_unknown() {
        assert!(parse_claude_stream_line("not json").is_none());
        assert!(parse_claude_stream_line(r#"{"type":"weird"}"#).is_none());
        assert!(parse_claude_stream_line("").is_none());
        assert!(parse_claude_stream_line(r#"{"type":"text"}"#).is_none()); // text 없음
    }

    #[test]
    fn escalation_marker_in_text() {
        let line = r#"{"type":"text","text":"진행 중 <<LUIDA_ASK category=design_mismatch>>스키마 충돌, 어느 쪽?<<END>> 입니다"}"#;
        let ev = parse_claude_stream_line(line).unwrap();
        assert_eq!(
            ev,
            AgentEvent::Escalation {
                category: "design_mismatch".into(),
                message: "스키마 충돌, 어느 쪽?".into()
            }
        );
    }

    #[test]
    fn detect_escalation_standalone() {
        assert!(detect_escalation("그냥 텍스트").is_none());
        let (c, m) = detect_escalation("<<LUIDA_ASK category=dangerous_op>>rm -rf?<<END>>").unwrap();
        assert_eq!(c, "dangerous_op");
        assert_eq!(m, "rm -rf?");
    }

    #[test]
    fn detect_escalation_requires_end_marker() {
        // <<END>> 없으면 None (텍스트 끝까지 흡수 방지)
        assert!(detect_escalation("<<LUIDA_ASK category=design_mismatch>>질문 계속...").is_none());
    }

    #[test]
    fn detect_escalation_rejects_empty_category() {
        assert!(detect_escalation("<<LUIDA_ASK category=>>x<<END>>").is_none());
    }

    #[test]
    fn scripted_runtime_success() {
        let rt = ScriptedRuntime::new(vec![
            AgentEvent::Text { text: "work".into() },
            AgentEvent::ToolUse { name: "Write".into() },
            AgentEvent::Result {
                success: true,
                summary: Some("ok".into()),
            },
        ]);
        let mut seen = 0;
        let outcome = rt
            .run("m", &AgentInvocation::default(), &mut |_| seen += 1)
            .unwrap();
        assert_eq!(seen, 3);
        assert!(outcome.success);
        assert!(outcome.saw_result);
        assert_eq!(outcome.summary.as_deref(), Some("ok"));
    }

    #[test]
    fn scripted_runtime_no_result_is_failure() {
        let rt = ScriptedRuntime::new(vec![
            AgentEvent::Text { text: "started".into() },
            AgentEvent::Error {
                message: "crash".into(),
            },
        ]);
        let outcome = rt
            .run("m", &AgentInvocation::default(), &mut |_| {})
            .unwrap();
        assert!(!outcome.success);
        assert!(!outcome.saw_result);
        // Error message가 summary로
        assert_eq!(outcome.summary.as_deref(), Some("crash"));
    }

    #[test]
    fn scripted_runtime_captures_escalation() {
        let rt = ScriptedRuntime::new(vec![
            AgentEvent::Escalation {
                category: "ambiguous_spec".into(),
                message: "어느 API?".into(),
            },
            AgentEvent::Result {
                success: true,
                summary: None,
            },
        ]);
        let outcome = rt
            .run("m", &AgentInvocation::default(), &mut |_| {})
            .unwrap();
        assert_eq!(
            outcome.escalation,
            Some(("ambiguous_spec".into(), "어느 API?".into()))
        );
    }
}
