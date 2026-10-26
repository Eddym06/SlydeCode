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
    /// Provider round trips and token counts for the turn. `estimated` is true
    /// when the provider reported no usage and the numbers are a size estimate.
    Usage {
        calls: u32,
        input_tokens: u64,
        output_tokens: u64,
        estimated: bool,
    },
    /// A tool call is still being written by the model: how much of its
    /// arguments has arrived and how many slides they carry so far.
    Progress {
        name: String,
        chars: usize,
        items: usize,
    },
    /// The turn changed the deck and can be undone in one step.
    Reversible,
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

/// Tokens a provider reported for one request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
}

impl TokenUsage {
    pub fn is_reported(&self) -> bool {
        self.input > 0 || self.output > 0
    }
}
