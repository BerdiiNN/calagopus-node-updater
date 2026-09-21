use shared::State;

pub mod admin;

/// A `DisplayError`-carrying `anyhow::Error` that maps to a specific HTTP status when a route
/// returns it.
pub(crate) fn err(status: axum::http::StatusCode, message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(shared::response::DisplayError::new(message.into()).with_status(status))
}
