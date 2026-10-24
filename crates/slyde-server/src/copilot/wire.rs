//! Wire format between the provider loop and the SSE endpoint.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    Meta { provider: String, model: String },
    Delta { text: String },
    Thinking { text: String },
    Tool { name: String, status: String },
    Done,
    Error { code: String, message: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImagePart {
    pub mime: String,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Turn {
    User {
        text: String,
        images: Vec<ImagePart>,
    },
    Assistant {
        text: String,
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        call_id: String,
        name: String,
        content: String,
    },
}

impl Turn {
    pub fn user(text: impl Into<String>) -> Self {
        Self::User {
            text: text.into(),
            images: Vec::new(),
        }
    }
}
