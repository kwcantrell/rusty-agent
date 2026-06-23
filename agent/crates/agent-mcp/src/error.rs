#[derive(Debug, Clone, thiserror::Error)]
pub enum McpError {
    #[error("io error: {0}")]
    Io(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("request timed out")]
    Timeout,
    #[error("server returned error: {0}")]
    Server(String),
    #[error("transport closed")]
    Closed,
}
