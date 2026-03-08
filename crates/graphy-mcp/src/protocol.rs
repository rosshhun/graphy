use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── JSON-RPC Types ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

impl JsonRpcResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<Value>, code: i64, message: String) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError { code, message }),
        }
    }
}

/// A JSON-RPC notification (no id, no response expected).
#[derive(Debug, Serialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcNotification {
    pub fn new(method: &str, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        }
    }
}

// ── MCP Protocol Types ──────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    #[serde(rename = "serverInfo")]
    pub server_info: ServerInfo,
}

#[derive(Debug, Serialize)]
pub struct ServerCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,
}

#[derive(Debug, Serialize)]
pub struct ToolsCapability {}

#[derive(Debug, Serialize)]
pub struct ResourcesCapability {}

#[derive(Debug, Serialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Debug, Serialize)]
pub struct ToolsListResult {
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Deserialize)]
pub struct CallToolParams {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Debug, Serialize)]
pub struct CallToolResult {
    pub content: Vec<ContentBlock>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

impl CallToolResult {
    pub fn text(text: String) -> Self {
        Self {
            content: vec![ContentBlock {
                content_type: "text".into(),
                text,
            }],
            is_error: None,
        }
    }

    pub fn error(message: String) -> Self {
        Self {
            content: vec![ContentBlock {
                content_type: "text".into(),
                text: message,
            }],
            is_error: Some(true),
        }
    }
}

// ── MCP Resources ───────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ResourceDefinition {
    pub uri: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ResourcesListResult {
    pub resources: Vec<ResourceDefinition>,
}

#[derive(Debug, Deserialize)]
pub struct ResourceReadParams {
    pub uri: String,
}

#[derive(Debug, Serialize)]
pub struct ResourceContent {
    pub uri: String,
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct ResourceReadResult {
    pub contents: Vec<ResourceContent>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── JSON-RPC Request parsing ──────────────────────────

    #[test]
    fn parse_valid_request() {
        let json = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        });
        let req: JsonRpcRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.method, "initialize");
        assert_eq!(req.id, Some(json!(1)));
    }

    #[test]
    fn parse_request_without_id() {
        // Notifications have no id
        let json = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });
        let req: JsonRpcRequest = serde_json::from_value(json).unwrap();
        assert!(req.id.is_none());
    }

    #[test]
    fn parse_request_without_params() {
        let json = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list"
        });
        let req: JsonRpcRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.params, Value::Null);
    }

    #[test]
    fn parse_request_string_id() {
        let json = json!({
            "jsonrpc": "2.0",
            "id": "abc-123",
            "method": "tools/call",
            "params": {}
        });
        let req: JsonRpcRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.id, Some(json!("abc-123")));
    }

    // ── JSON-RPC Response construction ────────────────────

    #[test]
    fn success_response_serialization() {
        let resp = JsonRpcResponse::success(Some(json!(1)), json!({"status": "ok"}));
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 1);
        assert!(json["result"].is_object());
        assert!(json.get("error").is_none());
    }

    #[test]
    fn error_response_serialization() {
        let resp = JsonRpcResponse::error(Some(json!(2)), -32600, "Invalid Request".into());
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["error"]["code"], -32600);
        assert_eq!(json["error"]["message"], "Invalid Request");
        assert!(json.get("result").is_none());
    }

    #[test]
    fn response_null_id() {
        let resp = JsonRpcResponse::success(None, json!("ok"));
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json.get("id").is_none());
    }

    // ── Notification ──────────────────────────────────────

    #[test]
    fn notification_construction() {
        let notif = JsonRpcNotification::new("notifications/resources/updated", Some(json!({"uri": "graphy://health"})));
        assert_eq!(notif.jsonrpc, "2.0");
        assert_eq!(notif.method, "notifications/resources/updated");
        assert!(notif.params.is_some());
    }

    #[test]
    fn notification_without_params() {
        let notif = JsonRpcNotification::new("initialized", None);
        let json = serde_json::to_value(&notif).unwrap();
        assert!(json.get("params").is_none());
    }

    // ── MCP Types ─────────────────────────────────────────

    #[test]
    fn call_tool_result_text() {
        let result = CallToolResult::text("hello".into());
        assert_eq!(result.content.len(), 1);
        assert_eq!(result.content[0].content_type, "text");
        assert_eq!(result.content[0].text, "hello");
        assert!(result.is_error.is_none());
    }

    #[test]
    fn call_tool_result_error() {
        let result = CallToolResult::error("something went wrong".into());
        assert_eq!(result.is_error, Some(true));
        assert_eq!(result.content[0].text, "something went wrong");
    }

    #[test]
    fn call_tool_params_deserialization() {
        let json = json!({
            "name": "graphy_query",
            "arguments": {"mode": "search", "query": "main"}
        });
        let params: CallToolParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.name, "graphy_query");
        assert_eq!(params.arguments["mode"], "search");
    }

    #[test]
    fn call_tool_params_empty_arguments() {
        let json = json!({
            "name": "graphy_analyze"
        });
        let params: CallToolParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.arguments, Value::Null);
    }

    #[test]
    fn initialize_result_serialization() {
        let result = InitializeResult {
            protocol_version: "2024-11-05".into(),
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability {}),
                resources: Some(ResourcesCapability {}),
            },
            server_info: ServerInfo {
                name: "graphy".into(),
                version: "1.0.0".into(),
            },
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["protocolVersion"], "2024-11-05");
        assert_eq!(json["serverInfo"]["name"], "graphy");
        assert!(json["capabilities"]["tools"].is_object());
    }

    #[test]
    fn resource_definition_serialization() {
        let res = ResourceDefinition {
            uri: "graphy://architecture".into(),
            name: "Architecture".into(),
            description: Some("Overview".into()),
            mime_type: Some("text/plain".into()),
        };
        let json = serde_json::to_value(&res).unwrap();
        assert_eq!(json["uri"], "graphy://architecture");
        assert_eq!(json["mimeType"], "text/plain");
    }
}
