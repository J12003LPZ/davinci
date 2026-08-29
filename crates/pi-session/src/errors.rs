use std::fmt;

#[derive(Debug, Clone)]
pub struct JsonlDecodeError {
    pub kind: &'static str,
    pub message: String,
}

impl JsonlDecodeError {
    pub fn syntax(message: impl Into<String>) -> Self {
        Self {
            kind: "syntax",
            message: message.into(),
        }
    }

    pub fn schema(message: impl Into<String>) -> Self {
        Self {
            kind: "schema",
            message: message.into(),
        }
    }
}

impl fmt::Display for JsonlDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for JsonlDecodeError {}

#[derive(Debug, Clone)]
pub struct SessionError {
    pub code: &'static str,
    pub message: String,
}

impl SessionError {
    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            code: "not_found",
            message: message.into(),
        }
    }

    pub fn storage(message: impl Into<String>) -> Self {
        Self {
            code: "storage",
            message: message.into(),
        }
    }

    pub fn invalid_entry(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_entry",
            message: message.into(),
        }
    }

    pub fn already_exists(message: impl Into<String>) -> Self {
        Self {
            code: "already_exists",
            message: message.into(),
        }
    }

    pub fn invalid_query(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_query",
            message: message.into(),
        }
    }

    pub fn invalid_lane(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_lane",
            message: message.into(),
        }
    }

    pub fn invalid_fork_target(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_fork_target",
            message: message.into(),
        }
    }
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SessionError {}
