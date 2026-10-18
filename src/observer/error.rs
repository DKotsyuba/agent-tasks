//! Safe classified observer failures; raw provider/source payloads never become diagnostics.

/// An observer refusal or unavailable dependency, safe to pass to the renderer.
#[derive(Debug, Clone)]
pub struct Error {
    /// Stable machine code used for recovery decisions.
    pub code: String,
    /// Safe explanation without untrusted parser excerpts or credentials.
    pub message: String,
}

impl Error {
    /// Construct a classified failure; callers supply a safe, non-secret explanation.
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Error {
    /// Format the safe code/message, without an underlying payload.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    /// Classify local I/O without disclosing paths or arbitrary system text.
    fn from(_: std::io::Error) -> Self {
        Self::new("io", "Observer storage or local transport is unavailable.")
    }
}

impl From<serde_json::Error> for Error {
    /// Reject malformed JSON without returning source excerpts.
    fn from(_: serde_json::Error) -> Self {
        Self::new(
            "invalid_json",
            "Invalid observer JSON; no result was accepted.",
        )
    }
}

/// Observer result with stable, presentation-safe errors.
pub type Result<T> = std::result::Result<T, Error>;
