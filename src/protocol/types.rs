#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── CLI → Server Messages (incoming NDJSON) ────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum CliMessage {
    #[serde(rename = "system")]
    System(SystemMessage),
    #[serde(rename = "assistant")]
    Assistant(AssistantMessage),
    #[serde(rename = "result")]
    Result(ResultMessage),
    #[serde(rename = "stream_event")]
    StreamEvent(StreamEventMessage),
    #[serde(rename = "control_request")]
    ControlRequest(ControlRequestMessage),
    #[serde(rename = "tool_progress")]
    ToolProgress(ToolProgressMessage),
    #[serde(rename = "tool_use_summary")]
    ToolUseSummary(ToolUseSummaryMessage),
    #[serde(rename = "auth_status")]
    AuthStatus(AuthStatusMessage),
    #[serde(rename = "message_history")]
    MessageHistory(MessageHistoryMessage),
    #[serde(rename = "keep_alive")]
    KeepAlive,
    #[serde(other)]
    Unknown,
}

// ─── System Messages ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct SystemMessage {
    pub subtype: String,
    pub session_id: Option<String>,
    pub uuid: Option<String>,
    // init fields
    pub cwd: Option<String>,
    pub tools: Option<Vec<String>>,
    pub mcp_servers: Option<Vec<McpServer>>,
    pub model: Option<String>,
    #[serde(rename = "permissionMode")]
    pub permission_mode: Option<String>,
    #[serde(rename = "apiKeySource")]
    pub api_key_source: Option<String>,
    pub claude_code_version: Option<String>,
    pub slash_commands: Option<Vec<String>>,
    pub agents: Option<Vec<String>>,
    pub skills: Option<Vec<String>>,
    pub output_style: Option<String>,
    // status fields
    pub status: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpServer {
    pub name: String,
    pub status: String,
}

// ─── Assistant Message ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct AssistantMessage {
    pub message: AssistantMessageBody,
    pub parent_tool_use_id: Option<String>,
    pub error: Option<String>,
    pub uuid: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssistantMessageBody {
    pub id: String,
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    pub role: Option<String>,
    pub model: Option<String>,
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub usage: Option<Usage>,
}

// ─── Content Blocks ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: serde_json::Value,
        #[serde(default)]
        is_error: bool,
    },
    #[serde(rename = "thinking")]
    Thinking {
        thinking: String,
        #[serde(default)]
        budget_tokens: Option<u32>,
    },
    #[serde(other)]
    Unknown,
}

// ─── Result Message ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ResultMessage {
    pub subtype: String,
    pub is_error: bool,
    pub result: Option<String>,
    pub errors: Option<Vec<String>>,
    pub duration_ms: Option<f64>,
    pub duration_api_ms: Option<f64>,
    pub num_turns: Option<u32>,
    pub total_cost_usd: Option<f64>,
    pub stop_reason: Option<String>,
    pub usage: Option<Usage>,
    #[serde(rename = "modelUsage")]
    pub model_usage: Option<HashMap<String, ModelUsage>>,
    pub uuid: Option<String>,
    pub session_id: Option<String>,
    pub total_lines_added: Option<u32>,
    pub total_lines_removed: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelUsage {
    #[serde(rename = "inputTokens")]
    pub input_tokens: Option<u64>,
    #[serde(rename = "outputTokens")]
    pub output_tokens: Option<u64>,
    #[serde(rename = "cacheReadInputTokens")]
    pub cache_read_input_tokens: Option<u64>,
    #[serde(rename = "cacheCreationInputTokens")]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(rename = "contextWindow")]
    pub context_window: Option<u64>,
    #[serde(rename = "maxOutputTokens")]
    pub max_output_tokens: Option<u64>,
    #[serde(rename = "costUSD")]
    pub cost_usd: Option<f64>,
}

// ─── Stream Event ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct StreamEventMessage {
    pub event: serde_json::Value,
    pub parent_tool_use_id: Option<String>,
    pub uuid: Option<String>,
    pub session_id: Option<String>,
}

// ─── Control Request (CLI → Server) ─────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ControlRequestMessage {
    pub request_id: String,
    pub request: ControlRequestPayload,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "subtype")]
pub enum ControlRequestPayload {
    #[serde(rename = "can_use_tool")]
    CanUseTool {
        tool_name: String,
        input: serde_json::Value,
        tool_use_id: String,
        description: Option<String>,
        permission_suggestions: Option<Vec<serde_json::Value>>,
        agent_id: Option<String>,
    },
    #[serde(rename = "hook_callback")]
    HookCallback {
        callback_id: String,
        input: serde_json::Value,
        tool_use_id: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

// ─── Tool Progress ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ToolProgressMessage {
    pub tool_use_id: String,
    pub tool_name: String,
    pub parent_tool_use_id: Option<String>,
    pub elapsed_time_seconds: Option<f64>,
    pub uuid: Option<String>,
    pub session_id: Option<String>,
}

// ─── Tool Use Summary ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ToolUseSummaryMessage {
    pub summary: String,
    pub preceding_tool_use_ids: Option<Vec<String>>,
    pub uuid: Option<String>,
    pub session_id: Option<String>,
}

// ─── Auth Status ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct AuthStatusMessage {
    #[serde(rename = "isAuthenticating")]
    pub is_authenticating: bool,
    pub output: Option<Vec<String>>,
    pub error: Option<String>,
    pub uuid: Option<String>,
    pub session_id: Option<String>,
}

// ─── Server → CLI Messages (outgoing NDJSON) ────────────────────────────────

/// User message sent to the CLI
#[derive(Debug, Serialize)]
pub struct OutgoingUserMessage {
    #[serde(rename = "type")]
    pub msg_type: &'static str,
    pub message: OutgoingUserContent,
    pub parent_tool_use_id: Option<String>,
    pub session_id: String,
}

#[derive(Debug, Serialize)]
pub struct OutgoingUserContent {
    pub role: &'static str,
    pub content: String,
}

impl OutgoingUserMessage {
    pub fn new(content: String, session_id: String) -> Self {
        Self {
            msg_type: "user",
            message: OutgoingUserContent {
                role: "user",
                content,
            },
            parent_tool_use_id: None,
            session_id,
        }
    }

    pub fn to_ndjson(&self) -> String {
        serde_json::to_string(self).expect("serialization of protocol message cannot fail")
    }
}

/// User message with image content sent to the CLI
#[derive(Debug, Serialize)]
pub struct OutgoingImageMessage {
    #[serde(rename = "type")]
    pub msg_type: &'static str,
    pub message: OutgoingImageContent,
    pub session_id: String,
}

#[derive(Debug, Serialize)]
pub struct OutgoingImageContent {
    pub role: &'static str,
    pub content: Vec<serde_json::Value>,
}

impl OutgoingImageMessage {
    pub fn new(text: Option<String>, image_b64: String, media_type: String, session_id: String) -> Self {
        let mut content = Vec::new();
        if let Some(t) = text {
            content.push(serde_json::json!({"type": "text", "text": t}));
        }
        content.push(serde_json::json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": media_type,
                "data": image_b64,
            }
        }));
        Self {
            msg_type: "user",
            message: OutgoingImageContent {
                role: "user",
                content,
            },
            session_id,
        }
    }

    pub fn to_ndjson(&self) -> String {
        serde_json::to_string(self).expect("serialization of protocol message cannot fail")
    }
}

/// Control response for permission requests
#[derive(Debug, Serialize)]
pub struct OutgoingControlResponse {
    #[serde(rename = "type")]
    pub msg_type: &'static str,
    pub response: ControlResponseBody,
}

#[derive(Debug, Serialize)]
pub struct ControlResponseBody {
    pub subtype: &'static str,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<serde_json::Value>,
}

impl OutgoingControlResponse {
    pub fn allow(request_id: String, updated_input: serde_json::Value) -> Self {
        Self {
            msg_type: "control_response",
            response: ControlResponseBody {
                subtype: "success",
                request_id,
                response: Some(serde_json::json!({
                    "behavior": "allow",
                    "updatedInput": updated_input,
                })),
            },
        }
    }

    pub fn deny(request_id: String, message: &str) -> Self {
        Self {
            msg_type: "control_response",
            response: ControlResponseBody {
                subtype: "success",
                request_id,
                response: Some(serde_json::json!({
                    "behavior": "deny",
                    "message": message,
                })),
            },
        }
    }

    pub fn to_ndjson(&self) -> String {
        serde_json::to_string(self).expect("serialization of protocol message cannot fail")
    }
}

/// Control request sent to the CLI (e.g., interrupt)
#[derive(Debug, Serialize)]
pub struct OutgoingControlRequest {
    #[serde(rename = "type")]
    pub msg_type: &'static str,
    pub request_id: String,
    pub request: serde_json::Value,
}

impl OutgoingControlRequest {
    pub fn interrupt() -> Self {
        Self {
            msg_type: "control_request",
            request_id: uuid::Uuid::new_v4().to_string(),
            request: serde_json::json!({ "subtype": "interrupt" }),
        }
    }

    pub fn to_ndjson(&self) -> String {
        serde_json::to_string(self).expect("serialization of protocol message cannot fail")
    }
}

// ─── Message History ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct MessageHistoryMessage {
    pub messages: Vec<HistoryEntry>,
    pub uuid: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryEntry {
    pub role: Option<String>,
    pub content: serde_json::Value,
    pub model: Option<String>,
}

// ─── Server → CLI Messages (outgoing NDJSON) ────────────────────────────────

/// Set permission mode message sent to the CLI
#[derive(Debug, Serialize)]
pub struct OutgoingSetPermissionMode {
    #[serde(rename = "type")]
    pub msg_type: &'static str,
    pub mode: String,
    pub session_id: String,
}

impl OutgoingSetPermissionMode {
    pub fn new(mode: String, session_id: String) -> Self {
        Self {
            msg_type: "set_permission_mode",
            mode,
            session_id,
        }
    }

    pub fn to_ndjson(&self) -> String {
        serde_json::to_string(self).expect("serialization of protocol message cannot fail")
    }
}

// ─── Helper functions ───────────────────────────────────────────────────────

/// Extract text from content blocks
pub fn extract_text_from_blocks(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            ContentBlock::Thinking { thinking, .. } => Some(thinking.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Format a tool use summary for display
pub fn format_tool_summary(name: &str, input: &serde_json::Value) -> String {
    match name {
        "Bash" => input
            .get("command")
            .and_then(|v| v.as_str())
            .map(|s| truncate_str(s, 120))
            .unwrap_or_default(),
        "Read" | "Write" => input
            .get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "Edit" => input
            .get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "Glob" => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "Grep" => input
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "Task" => input
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        _ => {
            let s = serde_json::to_string(input).unwrap_or_default();
            truncate_str(&s, 100)
        }
    }
}

/// Truncate a string to at most `max_chars` characters (UTF-8 safe)
fn truncate_str(s: &str, max_chars: usize) -> String {
    let boundary = s
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    if boundary < s.len() {
        format!("{}...", &s[..boundary])
    } else {
        s.to_string()
    }
}

/// Extract text from a tool_result content field (can be string or array)
pub fn extract_tool_result_text(content: &serde_json::Value) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    if let Some(arr) = content.as_array() {
        return arr
            .iter()
            .filter_map(|item| {
                if item.get("type")?.as_str()? == "text" {
                    item.get("text")?.as_str().map(String::from)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    String::new()
}
