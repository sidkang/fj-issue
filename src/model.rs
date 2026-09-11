use serde::Serialize;
use time::format_description::well_known::Rfc3339;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RelIssue {
    pub number: i64,
    pub state: String,
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CommentView {
    pub id: i64,
    pub author: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct IssueView {
    pub number: i64,
    pub url: String,
    pub title: String,
    pub body: String,
    pub state: String,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub author: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<Vec<RelIssue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks: Option<Vec<RelIssue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<Vec<CommentView>>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ListResult {
    pub items: Vec<IssueView>,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CommentResult {
    pub id: i64,
    pub issue: i64,
    pub author: String,
    pub body: String,
    pub created_at: String,
}

pub fn format_time(ts: Option<time::OffsetDateTime>) -> String {
    ts.and_then(|t| t.format(&Rfc3339).ok()).unwrap_or_default()
}
