use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMetadata {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub session_id: String,
    pub role: Role,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriterLease {
    pub session_id: String,
    pub holder_id: String,
    pub acquired_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcRequest<T = serde_json::Value> {
    pub id: String,
    pub method: String,
    pub params: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcResponse<T = serde_json::Value> {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    #[serde(rename_all = "camelCase")]
    Started { session_id: String, timestamp: i64 },
    #[serde(rename_all = "camelCase")]
    MessageChunk { session_id: String, chunk: String },
    #[serde(rename_all = "camelCase")]
    ToolCallStart {
        session_id: String,
        tool_call: ToolCall,
    },
    #[serde(rename_all = "camelCase")]
    ToolCallEnd {
        session_id: String,
        tool_call_id: String,
        result: String,
    },
    #[serde(rename_all = "camelCase")]
    Completed { session_id: String, timestamp: i64 },
    #[serde(rename_all = "camelCase")]
    Error { session_id: String, error: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_metadata_serde_parity() {
        let meta = SessionMetadata {
            id: "sess-123".to_string(),
            title: "Test Session".to_string(),
            created_at: 1000,
            updated_at: 2000,
            tags: vec!["tag1".to_string()],
        };

        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("\"createdAt\":1000"));
        assert!(json.contains("\"updatedAt\":2000"));

        let deserialized: SessionMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(meta, deserialized);
    }

    #[test]
    fn test_message_serde_parity() {
        let msg = Message {
            id: "msg-1".to_string(),
            session_id: "sess-1".to_string(),
            role: Role::User,
            content: "Hello".to_string(),
            tool_calls: None,
            tool_call_id: None,
            timestamp: 123456,
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"sessionId\":\"sess-1\""));

        let deserialized: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
    }
}
