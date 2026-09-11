use std::io::{self, Write};

use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum FjiError {
    #[error("{error}")]
    Usage { code: &'static str, error: String },
    #[error("{error}")]
    NotFound { error: String, http: Option<u16> },
    #[error("{error}")]
    Blocked {
        error: String,
        blocked_by: Option<Vec<i64>>,
    },
    #[error("{error}")]
    Conflict { error: String, http: Option<u16> },
    #[error("{error}")]
    Http {
        code: &'static str,
        error: String,
        http: Option<u16>,
    },
    #[error("{error}")]
    PartialCreate {
        error: String,
        issue_number: i64,
        issue_url: String,
        applied: Vec<i64>,
        failed: FailedDep,
    },
    #[error("{error}")]
    PartialDep {
        error: String,
        issue_number: i64,
        issue_url: String,
        applied: Vec<i64>,
        failed: FailedDep,
    },
    #[error("{error}")]
    Uncertain {
        error: String,
        issue_number: Option<i64>,
        issue_url: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct FailedDep {
    pub number: i64,
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http: Option<u16>,
}

impl FjiError {
    pub fn usage(code: &'static str, error: impl Into<String>) -> Self {
        Self::Usage {
            code,
            error: error.into(),
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Usage { .. } => 1,
            Self::Http { .. }
            | Self::PartialCreate { .. }
            | Self::PartialDep { .. }
            | Self::Uncertain { .. } => 2,
            Self::NotFound { .. } => 3,
            Self::Blocked { .. } | Self::Conflict { .. } => 4,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        map.insert("error".into(), serde_json::Value::String(self.to_string()));
        match self {
            Self::Usage { code, .. } => {
                map.insert("code".into(), serde_json::Value::String((*code).into()));
            }
            Self::NotFound { http, .. } => {
                map.insert("code".into(), "not_found".into());
                if let Some(http) = http {
                    map.insert("http".into(), serde_json::Value::Number((*http).into()));
                }
            }
            Self::Blocked { blocked_by, .. } => {
                map.insert("code".into(), "blocked".into());
                map.insert("http".into(), 412.into());
                if let Some(blocked_by) = blocked_by {
                    map.insert(
                        "blocked_by".into(),
                        serde_json::to_value(blocked_by).unwrap_or(serde_json::Value::Null),
                    );
                }
            }
            Self::Conflict { http, .. } => {
                map.insert("code".into(), "conflict".into());
                if let Some(http) = http {
                    map.insert("http".into(), serde_json::Value::Number((*http).into()));
                }
            }
            Self::Http { code, http, .. } => {
                map.insert("code".into(), serde_json::Value::String((*code).into()));
                if let Some(http) = http {
                    map.insert("http".into(), serde_json::Value::Number((*http).into()));
                }
            }
            Self::PartialCreate {
                issue_number,
                issue_url,
                applied,
                failed,
                ..
            } => {
                map.insert("code".into(), "partial_create".into());
                map.insert(
                    "issue".into(),
                    serde_json::json!({ "number": issue_number, "url": issue_url }),
                );
                map.insert(
                    "applied".into(),
                    serde_json::to_value(applied).unwrap_or(serde_json::Value::Null),
                );
                map.insert(
                    "failed".into(),
                    serde_json::to_value(failed).unwrap_or(serde_json::Value::Null),
                );
            }
            Self::PartialDep {
                issue_number,
                issue_url,
                applied,
                failed,
                ..
            } => {
                map.insert("code".into(), "partial_dep".into());
                map.insert(
                    "issue".into(),
                    serde_json::json!({ "number": issue_number, "url": issue_url }),
                );
                map.insert(
                    "applied".into(),
                    serde_json::to_value(applied).unwrap_or(serde_json::Value::Null),
                );
                map.insert(
                    "failed".into(),
                    serde_json::to_value(failed).unwrap_or(serde_json::Value::Null),
                );
            }
            Self::Uncertain {
                issue_number,
                issue_url,
                ..
            } => {
                map.insert("code".into(), "uncertain".into());
                if issue_number.is_some() || issue_url.is_some() {
                    map.insert(
                        "issue".into(),
                        serde_json::json!({ "number": issue_number, "url": issue_url }),
                    );
                }
            }
        }
        serde_json::Value::Object(map)
    }

    pub fn write_stderr(&self) {
        let payload = self.to_json();
        let mut stderr = io::stderr().lock();
        let _ = serde_json::to_writer(&mut stderr, &payload);
        let _ = stderr.write_all(b"\n");
    }
}
