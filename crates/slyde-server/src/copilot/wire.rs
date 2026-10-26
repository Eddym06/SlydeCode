//! Wire format between the provider loop and the SSE endpoint.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    Meta {
        provider: String,
        model: String,
    },
    Delta {
        text: String,
    },
    Thinking {
        text: String,
    },
    Tool {
        name: String,
        status: String,
    },
    /// Provider round trips and a rough input-size estimate for the turn.
    Usage {
        calls: u32,
        approx_input_tokens: u64,
    },
    Done,
    Error {
        code: String,
        message: String,
    },
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
    /// Rough character weight of a turn, used for the usage estimate.
    pub fn chars(&self) -> usize {
        match self {
            Turn::User { text, .. } => text.len(),
            Turn::Assistant { text, tool_calls } => {
                text.len()
                    + tool_calls
                        .iter()
                        .map(|call| call.name.len() + call.arguments.to_string().len())
                        .sum::<usize>()
            }
            Turn::Tool { content, .. } => content.len(),
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self::User {
            text: text.into(),
            images: Vec::new(),
        }
    }
}
