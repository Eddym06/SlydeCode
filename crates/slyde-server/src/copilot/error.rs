//! Errors that can cross the copilot API boundary.
//!
//! Provider payloads are untrusted and often contain far more detail than a
//! person can act on. The API exposes a stable code plus a short English
//! message; the UI turns the code into localized, actionable copy.

#[derive(Debug, thiserror::Error)]
pub enum CopilotError {
    #[error("no API key configured for this provider")]
    MissingKey,
    #[error("the provider rejected this API key")]
    InvalidKey,
    #[error("the provider is rate limiting requests; try again shortly")]
    RateLimited,
    #[error("the provider could not be reached")]
    Network,
    #[error("{0}")]
    Upstream(String),
    #[error("{0}")]
    InvalidRequest(String),
    #[error("this model does not support the capability the copilot needs")]
    Unsupported,
    #[error("{0}")]
    ToolLimit(String),
    #[error("generation cancelled")]
    Cancelled,
}

impl CopilotError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingKey => "missing_key",
            Self::InvalidKey => "invalid_key",
            Self::RateLimited => "rate_limited",
            Self::Network => "network",
            Self::Upstream(_) => "upstream",
            Self::InvalidRequest(_) => "invalid_request",
            Self::Unsupported => "unsupported",
            Self::ToolLimit(_) => "tool_limit",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn public_message(&self) -> String {
        match self {
            Self::Upstream(detail) => {
                let first = detail.lines().next().unwrap_or("").trim();
                if first.is_empty() {
                    "the provider rejected the request".to_string()
                } else {
                    // Provider error messages are documented API surface, but
                    // trim them so a stack trace never lands in a chat bubble.
                    first.chars().take(280).collect()
                }
            }
            Self::InvalidRequest(detail) => detail.clone(),
            Self::ToolLimit(detail) => detail.clone(),
            other => other.to_string(),
        }
    }

    /// Maps an upstream HTTP status before the body is interpreted.
    pub fn from_status(status: u16, body: &str) -> Self {
        if status == 401 || status == 403 {
            return Self::InvalidKey;
        }
        if status == 429 {
            return Self::RateLimited;
        }
        // A 4xx that complains about tools means the model cannot call them;
        // "try another model" is then the actionable advice, not a retry.
        let lower = body.to_ascii_lowercase();
        if (400..500).contains(&status)
            && (lower.contains("tool") || lower.contains("function calling"))
        {
            return Self::Unsupported;
        }
        Self::Upstream(format!("HTTP {status}: {body}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_failures_do_not_echo_the_key() {
        let error = CopilotError::from_status(401, "Bearer sk-secret was rejected");
        assert_eq!(error.code(), "invalid_key");
        assert!(!error.public_message().contains("sk-secret"));
    }

    #[test]
    fn tool_related_rejections_point_at_the_model() {
        let error = CopilotError::from_status(
            400,
            r#"{"error":{"message":"tools are not supported by this model"}}"#,
        );
        assert_eq!(error.code(), "unsupported");

        let generic = CopilotError::from_status(400, r#"{"error":"bad request"}"#);
        assert_eq!(generic.code(), "upstream");
    }

    #[test]
    fn upstream_messages_are_shortened() {
        let error = CopilotError::Upstream("x".repeat(900));
        assert_eq!(error.public_message().chars().count(), 280);
    }
}
