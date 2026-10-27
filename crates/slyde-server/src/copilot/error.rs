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
    #[error("the provider could not be reached: {0}")]
    Network(String),
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
    /// The request is bigger than the model's context or the plan's tokens-per-minute.
    #[error("{0}")]
    TooLarge(String),
    /// The model started a tool call and produced something that does not parse.
    #[error("{0}")]
    BadToolCall(String),
    /// The model id does not exist, was retired, or is not enabled for this key.
    #[error("{0}")]
    ModelUnavailable(String),
    #[error("the provider reports that this account has no credits left")]
    NoCredits,
    /// A classified failure that also keeps what the provider actually said, so
    /// the user can open a report instead of guessing.
    #[error("{kind}")]
    Detailed {
        kind: Box<CopilotError>,
        status: Option<u16>,
        detail: String,
    },
}

/// Longest provider reply kept for the report.
const MAX_DETAIL_CHARS: usize = 4000;

impl CopilotError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingKey => "missing_key",
            Self::InvalidKey => "invalid_key",
            Self::RateLimited => "rate_limited",
            Self::Network(_) => "network",
            Self::Upstream(_) => "upstream",
            Self::InvalidRequest(_) => "invalid_request",
            Self::Unsupported => "unsupported",
            Self::ToolLimit(_) => "tool_limit",
            Self::Cancelled => "cancelled",
            Self::TooLarge(_) => "too_large",
            Self::BadToolCall(_) => "bad_tool_call",
            Self::ModelUnavailable(_) => "model_unavailable",
            Self::NoCredits => "no_credits",
            Self::Detailed { kind, .. } => kind.code(),
        }
    }

    /// The classified error without the report wrapper.
    pub fn kind(&self) -> &CopilotError {
        match self {
            Self::Detailed { kind, .. } => kind.kind(),
            other => other,
        }
    }

    /// What the provider replied, redacted and bounded, when there is anything.
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Detailed { detail, .. } => Some(detail),
            Self::Upstream(detail) | Self::Network(detail) => Some(detail),
            _ => None,
        }
    }

    /// The HTTP status the provider answered with, when it answered.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Detailed { status, .. } => *status,
            _ => None,
        }
    }

    /// True when the provider objected to a request field by name, e.g.
    /// `stream_options`, so the caller can retry without it.
    pub fn rejects_param(&self, name: &str) -> bool {
        self.detail()
            .is_some_and(|detail| detail.to_ascii_lowercase().contains(name))
    }

    pub fn is_too_large(&self) -> bool {
        matches!(self.kind(), Self::TooLarge(_))
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
            // A bare "the provider could not be reached" hides the difference
            // between DNS, TLS, a proxy and a timeout. Keep the cause, on one
            // line and bounded, so the user can act on it.
            Self::Network(cause) => {
                let first = cause.lines().next().unwrap_or("").trim();
                if first.is_empty() {
                    "the provider could not be reached".to_string()
                } else {
                    let detail: String = first.chars().take(200).collect();
                    format!("the provider could not be reached: {detail}")
                }
            }
            Self::InvalidRequest(detail) => detail.clone(),
            Self::ToolLimit(detail) => detail.clone(),
            Self::Detailed { kind, .. } => kind.public_message(),
            other => other.to_string(),
        }
    }

    /// Maps an upstream HTTP status and body, keeping both for the report.
    pub fn from_status(status: u16, body: &str) -> Self {
        Self::from_status_redacting(status, body, "")
    }

    /// Like [`from_status`](Self::from_status), with `secret` (the API key) removed
    /// from the kept text in case a provider echoes it back.
    pub fn from_status_redacting(status: u16, body: &str, secret: &str) -> Self {
        let lower = body.to_ascii_lowercase();
        let has = |needles: &[&str]| needles.iter().any(|needle| lower.contains(needle));

        let kind = if has(&[
            "tool_use_failed",
            "failed to call a function",
            "failed_generation",
        ]) {
            // The model tried and wrote an invalid call: not "this model cannot
            // use tools", and a second attempt often succeeds.
            Self::BadToolCall("the model wrote an invalid tool call".to_string())
        } else if status == 413
            || has(&[
                "tokens per minute",
                "request too large",
                "context length",
                "context_length",
                "maximum context",
                "too many tokens",
                "reduce the length",
                "input is too long",
                "prompt is too long",
            ])
        {
            Self::TooLarge("the request is larger than the model or plan allows".to_string())
        } else if status == 402 {
            Self::NoCredits
        } else if status == 401
            || (status == 403
                && !has(&[
                    "model",
                    "region",
                    "country",
                    "moderation",
                    "flagged",
                    "policy",
                ]))
        {
            Self::InvalidKey
        } else if status == 429 {
            Self::RateLimited
        } else if status == 404
            || (has(&["model"])
                && has(&[
                    "does not exist",
                    "not found",
                    "no endpoints found",
                    "not available",
                    "not enabled",
                    "does not have access",
                    "decommissioned",
                    "deprecated",
                    "not supported for",
                ]))
        {
            Self::ModelUnavailable("the model is not available for this key".to_string())
        } else if (400..500).contains(&status)
            && has(&["tool", "function calling"])
            && has(&["support", "enable", "choice", "not allowed", "unsupported"])
        {
            // The model cannot call tools; "pick another model" is the advice.
            Self::Unsupported
        } else {
            Self::Upstream(format!("HTTP {status}: {}", first_line(body)))
        };

        Self::Detailed {
            kind: Box::new(kind),
            status: Some(status),
            detail: bounded(&redact(body, secret)),
        }
    }

    /// Wraps a transport failure, keeping the whole cause chain.
    ///
    /// `reqwest` reports "error sending request for url (…)", which says nothing
    /// about *why* it failed; the source chain carries the actual cause (DNS,
    /// TLS, connection refused, timeout).
    pub fn from_network(error: &(impl std::error::Error + ?Sized)) -> Self {
        let mut parts = vec![error.to_string()];
        let mut source = error.source();
        while let Some(cause) = source {
            let text = cause.to_string();
            if !parts.iter().any(|part| part == &text) {
                parts.push(text);
            }
            source = cause.source();
        }
        Self::Network(parts.join(": "))
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("").trim()
}

/// Removes the API key and anything that looks like a bearer token.
fn redact(text: &str, secret: &str) -> String {
    let mut out = text.to_string();
    if secret.len() >= 8 {
        out = out.replace(secret, "[redacted]");
    }
    let mut cleaned = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(at) = rest.to_ascii_lowercase().find("bearer ") {
        let (head, tail) = rest.split_at(at + "bearer ".len());
        cleaned.push_str(head);
        let end = tail
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
            .unwrap_or(tail.len());
        cleaned.push_str("[redacted]");
        rest = &tail[end..];
    }
    cleaned.push_str(rest);
    cleaned
}

fn bounded(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= MAX_DETAIL_CHARS {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(MAX_DETAIL_CHARS).collect();
    format!("{cut}\n… (truncated)")
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
        assert!(generic.public_message().contains("HTTP 400"));
    }

    #[test]
    fn a_plan_limit_is_a_size_problem_not_an_unsupported_model() {
        let groq = r#"{"error":{"message":"Request too large for model `llama-3.1-8b-instant` on tokens per minute (TPM): Limit 6000, Requested 9421, please reduce your message size and try again.","type":"tokens","code":"rate_limit_exceeded"}}"#;
        let error = CopilotError::from_status(413, groq);
        assert_eq!(error.code(), "too_large");
        assert!(error.is_too_large());
        assert!(
            error.detail().unwrap().contains("Limit 6000"),
            "the report keeps the numbers"
        );
        assert_eq!(error.status(), Some(413));
        // A 429 that names tokens per minute is the same problem: retrying unchanged cannot help.
        assert_eq!(CopilotError::from_status(429, groq).code(), "too_large");
        assert_eq!(
            CopilotError::from_status(429, "slow down").code(),
            "rate_limited"
        );
    }

    #[test]
    fn a_malformed_tool_call_is_not_reported_as_an_unsupported_model() {
        let body = r#"{"error":{"message":"Failed to call a function. Please adjust your prompt.","type":"invalid_request_error","code":"tool_use_failed","failed_generation":"<function=upsert_slide>..."}}"#;
        assert_eq!(CopilotError::from_status(400, body).code(), "bad_tool_call");
    }

    #[test]
    fn retired_models_missing_credits_and_forbidden_models_are_told_apart() {
        let gone = r#"{"error":{"message":"No endpoints found that support tool use. Try disabling \"tools\"."}}"#;
        // OpenRouter says "tool" here but the problem is the model's routes.
        assert_eq!(
            CopilotError::from_status(404, gone).code(),
            "model_unavailable"
        );
        assert_eq!(
            CopilotError::from_status(402, "insufficient credits").code(),
            "no_credits"
        );
        assert_eq!(
            CopilotError::from_status(403, r#"{"error":"model not enabled for this account"}"#)
                .code(),
            "model_unavailable"
        );
        assert_eq!(
            CopilotError::from_status(403, "bad key").code(),
            "invalid_key"
        );
    }

    #[test]
    fn the_report_never_carries_the_key() {
        let key = "sk-live-1234567890abcdef";
        let error = CopilotError::from_status_redacting(
            400,
            &format!(r#"{{"error":"bad header Authorization: Bearer {key} and {key}"}}"#),
            key,
        );
        let detail = error.detail().unwrap();
        assert!(!detail.contains(key), "{detail}");
        assert!(detail.contains("[redacted]"));
    }

    #[test]
    fn a_param_the_provider_names_can_be_retried_without() {
        let error = CopilotError::from_status(
            400,
            r#"{"detail":"Extra inputs are not permitted: stream_options"}"#,
        );
        assert!(error.rejects_param("stream_options"));
        assert!(!error.rejects_param("tools"));
    }

    #[test]
    fn upstream_messages_are_shortened() {
        let error = CopilotError::Upstream("x".repeat(900));
        assert_eq!(error.public_message().chars().count(), 280);
    }
}
