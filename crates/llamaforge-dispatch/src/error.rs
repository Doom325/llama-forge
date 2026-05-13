use std::fmt;

/// HTTP Status code domain wrapper
///
/// See design doc Section 2.6
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpStatus(pub u16);

impl HttpStatus {
    pub fn is_server_error(&self) -> bool {
        self.0 >= 500 && self.0 < 600
    }

    pub fn is_client_error(&self) -> bool {
        self.0 >= 400 && self.0 < 500
    }
}

impl fmt::Display for HttpStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The phase during which a proxy task was cancelled.
///
/// See design doc Section 2.6
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationPhase {
    PreConnect,
    PreStream,
    Streaming,
    Cleanup,
}

impl fmt::Display for CancellationPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PreConnect => write!(f, "pre-connect"),
            Self::PreStream => write!(f, "pre-stream"),
            Self::Streaming => write!(f, "streaming"),
            Self::Cleanup => write!(f, "cleanup"),
        }
    }
}
