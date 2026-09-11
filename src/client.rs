use forgejo_api::structs::{
    CreateIssueCommentOption, CreateIssueOption, DeleteLabelsOption, EditIssueOption,
    Issue as ApiIssue, IssueGetCommentsQuery, IssueLabelsOption, IssueListIssuesQuery,
    IssueListIssuesQueryState, IssueListIssuesQueryType, IssueListLabelsQuery, IssueMeta,
    StateType,
};
use forgejo_api::{ApiErrorKind, Auth, Forgejo, ForgejoError};
use serde_json::Value;

use crate::config::{IssueRef, RepoName, Resolved};
use crate::error::{FailedDep, FjiError};
use crate::model::{CommentResult, CommentView, IssueView, ListResult, RelIssue, format_time};

const PAGE: u32 = 50;
const UA: &str = concat!("fj-issue/", env!("CARGO_PKG_VERSION"));

pub struct Client {
    api: Forgejo,
    repo: RepoName,
}

impl Client {
    pub fn connect(resolved: &Resolved) -> Result<Self, FjiError> {
        let api =
            Forgejo::with_user_agent(Auth::Token(&resolved.token), resolved.host_url.clone(), UA)
                .map_err(map_forgejo)?;
        Ok(Self {
            api,
            repo: resolved.repo.clone(),
        })
    }

    fn owner(&self) -> &str {
        &self.repo.owner
    }

    fn name(&self) -> &str {
        &self.repo.name
    }

    pub async fn view(&self, number: i64) -> Result<IssueView, FjiError> {
        let issue = self.get_issue(number).await?;
        let comments = self.comments(number).await?;
        let (blocked_by, blocks) = self.deps(number).await?;
        Ok(project_issue(
            &issue,
            Some(blocked_by),
            Some(blocks),
            Some(comments),
        ))
    }

    pub async fn comment(&self, number: i64, body: String) -> Result<CommentResult, FjiError> {
        let comment = self
            .api
            .issue_create_comment(
                self.owner(),
                self.name(),
                number,
                CreateIssueCommentOption {
                    body,
                    updated_at: None,
                },
            )
            .await
            .map_err(map_forgejo)?;
        Ok(CommentResult {
            id: comment.id.unwrap_or(0),
            issue: number,
            author: login(comment.user.as_ref()),
            body: comment.body.unwrap_or_default(),
            created_at: format_time(comment.created_at),
        })
    }

    pub async fn edit(
        &self,
        number: i64,
        title: Option<String>,
        body: Option<String>,
    ) -> Result<IssueView, FjiError> {
        let issue = self
            .api
            .issue_edit_issue(
                self.owner(),
                self.name(),
                number,
                EditIssueOption {
                    title,
                    body,
                    assignee: None,
                    assignees: None,
                    due_date: None,
                    milestone: None,
                    r#ref: None,
                    state: None,
                    unset_due_date: None,
                    updated_at: None,
                },
            )
            .await
            .map_err(map_forgejo)?;
        self.attach_deps(issue).await
    }

    pub async fn label_add(&self, number: i64, labels: Vec<String>) -> Result<IssueView, FjiError> {
        self.ensure_labels_exist(&labels).await?;
        let values = labels.into_iter().map(Value::String).collect::<Vec<_>>();
        self.api
            .issue_add_label(
                self.owner(),
                self.name(),
                number,
                IssueLabelsOption {
                    labels: Some(values),
                    updated_at: None,
                },
            )
            .await
            .map_err(map_forgejo)?;
        self.view_without_comments(number).await
    }

    pub async fn label_rm(&self, number: i64, labels: Vec<String>) -> Result<IssueView, FjiError> {
        self.ensure_labels_exist(&labels).await?;
        for label in labels {
            self.api
                .issue_remove_label(
                    self.owner(),
                    self.name(),
                    number,
                    &label,
                    DeleteLabelsOption { updated_at: None },
                )
                .await
                .map_err(map_forgejo)?;
        }
        self.view_without_comments(number).await
    }

    pub async fn assign(&self, number: i64, users: Vec<String>) -> Result<IssueView, FjiError> {
        let issue = self.get_issue(number).await?;
        let mut assignees = current_assignees(&issue);
        for user in users {
            if !assignees.iter().any(|a| a == &user) {
                assignees.push(user);
            }
        }
        self.set_assignees(number, assignees).await
    }

    pub async fn unassign(&self, number: i64, users: Vec<String>) -> Result<IssueView, FjiError> {
        let issue = self.get_issue(number).await?;
        let assignees = current_assignees(&issue)
            .into_iter()
            .filter(|a| !users.iter().any(|u| u == a))
            .collect();
        self.set_assignees(number, assignees).await
    }

    async fn set_assignees(
        &self,
        number: i64,
        assignees: Vec<String>,
    ) -> Result<IssueView, FjiError> {
        let issue = self
            .api
            .issue_edit_issue(
                self.owner(),
                self.name(),
                number,
                EditIssueOption {
                    assignees: Some(assignees),
                    assignee: None,
                    body: None,
                    due_date: None,
                    milestone: None,
                    r#ref: None,
                    state: None,
                    title: None,
                    unset_due_date: None,
                    updated_at: None,
                },
            )
            .await
            .map_err(map_forgejo)?;
        self.attach_deps(issue).await
    }

    pub async fn close(&self, number: i64) -> Result<IssueView, FjiError> {
        self.set_state(number, "closed").await
    }

    pub async fn reopen(&self, number: i64) -> Result<IssueView, FjiError> {
        self.set_state(number, "open").await
    }

    async fn set_state(&self, number: i64, state: &str) -> Result<IssueView, FjiError> {
        let result = self
            .api
            .issue_edit_issue(
                self.owner(),
                self.name(),
                number,
                EditIssueOption {
                    state: Some(state.into()),
                    assignee: None,
                    assignees: None,
                    body: None,
                    due_date: None,
                    milestone: None,
                    r#ref: None,
                    title: None,
                    unset_due_date: None,
                    updated_at: None,
                },
            )
            .await;
        match result {
            Ok(issue) => self.attach_deps(issue).await,
            Err(e) => {
                let mapped = map_forgejo(e);
                if matches!(
                    mapped,
                    FjiError::Blocked {
                        blocked_by: None,
                        ..
                    }
                ) {
                    let blocked_by = self
                        .deps(number)
                        .await
                        .ok()
                        .map(|(blocked, _)| blocked.into_iter().map(|r| r.number).collect());
                    return Err(FjiError::Blocked {
                        error: mapped.to_string(),
                        blocked_by,
                    });
                }
                Err(mapped)
            }
        }
    }

    pub async fn create(
        &self,
        title: String,
        body: String,
        labels: Vec<String>,
        assignees: Vec<String>,
        blocked_by: Vec<IssueRef>,
    ) -> Result<IssueView, FjiError> {
        let label_ids = self.resolve_label_ids(&labels).await?;
        let issue = self
            .api
            .issue_create_issue(
                self.owner(),
                self.name(),
                CreateIssueOption {
                    title,
                    body: Some(body),
                    assignee: None,
                    assignees: if assignees.is_empty() {
                        None
                    } else {
                        Some(assignees)
                    },
                    closed: None,
                    due_date: None,
                    labels: if label_ids.is_empty() {
                        None
                    } else {
                        Some(label_ids)
                    },
                    milestone: None,
                    r#ref: None,
                },
            )
            .await
            .map_err(map_forgejo)?;
        let number = issue.number.ok_or_else(|| FjiError::Http {
            code: "http",
            error: "created issue has no number".into(),
            http: None,
        })?;
        let url = issue_url(&issue);
        if blocked_by.is_empty() {
            return self.attach_deps(issue).await;
        }
        match self.add_deps_inner(number, &url, &blocked_by).await {
            Ok(()) => self.attach_deps(issue).await,
            Err(FjiError::PartialDep {
                error,
                issue_number,
                issue_url,
                applied,
                failed,
            }) => Err(FjiError::PartialCreate {
                error,
                issue_number,
                issue_url,
                applied,
                failed,
            }),
            Err(other) => Err(other),
        }
    }

    pub async fn dep_add(
        &self,
        number: i64,
        blocked_by: Vec<IssueRef>,
    ) -> Result<IssueView, FjiError> {
        let url = self.get_issue(number).await.map(|i| issue_url(&i))?;
        self.add_deps_inner(number, &url, &blocked_by).await?;
        self.view_without_comments(number).await
    }

    pub async fn dep_rm(
        &self,
        number: i64,
        blocked_by: Vec<IssueRef>,
    ) -> Result<IssueView, FjiError> {
        for target in blocked_by {
            self.api
                .issue_remove_issue_dependencies(
                    self.owner(),
                    self.name(),
                    number,
                    IssueMeta {
                        index: Some(target.number),
                        owner: Some(self.owner().into()),
                        repo: Some(self.name().into()),
                    },
                )
                .await
                .map_err(map_forgejo)?;
        }
        self.view_without_comments(number).await
    }

    pub async fn dep_list(&self, number: i64) -> Result<IssueView, FjiError> {
        self.view_without_comments(number).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list(
        &self,
        state: crate::cli::ListState,
        labels: Vec<String>,
        assignee: Option<String>,
        search: Option<String>,
        limit: u32,
        all: bool,
        no_deps: bool,
    ) -> Result<ListResult, FjiError> {
        let state = match state {
            crate::cli::ListState::Open => IssueListIssuesQueryState::Open,
            crate::cli::ListState::Closed => IssueListIssuesQueryState::Closed,
            crate::cli::ListState::All => IssueListIssuesQueryState::All,
        };
        let query = IssueListIssuesQuery {
            state: Some(state),
            labels: if labels.is_empty() {
                None
            } else {
                Some(labels.join(","))
            },
            q: search,
            r#type: Some(IssueListIssuesQueryType::Issues),
            milestones: None,
            since: None,
            before: None,
            created_by: None,
            assigned_by: assignee,
            mentioned_by: None,
            sort: None,
        };

        let (items, truncated, total_count) = if all {
            let items = self.list_all(query).await?;
            (items, false, None)
        } else {
            self.list_page(query, limit).await?
        };

        let mut views = Vec::with_capacity(items.len());
        for issue in items {
            if issue.pull_request.is_some() {
                continue;
            }
            let number = match issue.number {
                Some(n) => n,
                None => continue,
            };
            let view = if no_deps {
                project_issue(&issue, None, None, None)
            } else {
                let (blocked_by, blocks) = self.deps(number).await?;
                project_issue(&issue, Some(blocked_by), Some(blocks), None)
            };
            views.push(view);
        }
        Ok(ListResult {
            items: views,
            truncated,
            total_count,
        })
    }

    async fn list_page(
        &self,
        query: IssueListIssuesQuery,
        limit: u32,
    ) -> Result<(Vec<ApiIssue>, bool, Option<i64>), FjiError> {
        let (headers, mut items) = self
            .api
            .issue_list_issues(self.owner(), self.name(), query)
            .page(1)
            .page_size(limit + 1)
            .send()
            .await
            .map_err(map_forgejo)?;
        let truncated = items.len() as u32 > limit;
        if truncated {
            items.truncate(limit as usize);
        }
        Ok((items, truncated, headers.x_total_count))
    }

    async fn list_all(&self, query: IssueListIssuesQuery) -> Result<Vec<ApiIssue>, FjiError> {
        let mut page = 1u32;
        let mut all = Vec::new();
        loop {
            let (_, items) = self
                .api
                .issue_list_issues(self.owner(), self.name(), query.clone())
                .page(page)
                .page_size(PAGE)
                .send()
                .await
                .map_err(map_forgejo)?;
            let n = items.len() as u32;
            all.extend(items);
            if n < PAGE {
                break;
            }
            page += 1;
        }
        Ok(all)
    }

    async fn add_deps_inner(
        &self,
        number: i64,
        url: &str,
        blocked_by: &[IssueRef],
    ) -> Result<(), FjiError> {
        let mut applied = Vec::new();
        for target in blocked_by {
            let result = self
                .api
                .issue_create_issue_dependencies(
                    self.owner(),
                    self.name(),
                    number,
                    IssueMeta {
                        index: Some(target.number),
                        owner: Some(self.owner().into()),
                        repo: Some(self.name().into()),
                    },
                )
                .await;
            match result {
                Ok(_) => applied.push(target.number),
                Err(e) => {
                    if is_uncertain(&e) {
                        return Err(FjiError::Uncertain {
                            error: e.to_string(),
                            issue_number: Some(number),
                            issue_url: Some(url.to_string()),
                        });
                    }
                    let mapped = map_forgejo(e);
                    return Err(FjiError::PartialDep {
                        error: mapped.to_string(),
                        issue_number: number,
                        issue_url: url.to_string(),
                        applied,
                        failed: FailedDep {
                            number: target.number,
                            error: mapped.to_string(),
                            http: http_of(&mapped),
                        },
                    });
                }
            }
        }
        Ok(())
    }

    async fn view_without_comments(&self, number: i64) -> Result<IssueView, FjiError> {
        let issue = self.get_issue(number).await?;
        self.attach_deps(issue).await
    }

    async fn attach_deps(&self, issue: ApiIssue) -> Result<IssueView, FjiError> {
        let number = issue.number.unwrap_or(0);
        let (blocked_by, blocks) = self.deps(number).await?;
        Ok(project_issue(&issue, Some(blocked_by), Some(blocks), None))
    }

    async fn get_issue(&self, number: i64) -> Result<ApiIssue, FjiError> {
        self.api
            .issue_get_issue(self.owner(), self.name(), number)
            .await
            .map_err(map_forgejo)
    }

    async fn comments(&self, number: i64) -> Result<Vec<CommentView>, FjiError> {
        let mut page = 1u32;
        let mut all = Vec::new();
        loop {
            let (_, items) = self
                .api
                .issue_get_comments(
                    self.owner(),
                    self.name(),
                    number,
                    IssueGetCommentsQuery {
                        since: None,
                        before: None,
                    },
                )
                .page(page)
                .page_size(PAGE)
                .send()
                .await
                .map_err(map_forgejo)?;
            let n = items.len() as u32;
            all.extend(items.into_iter().map(|c| CommentView {
                id: c.id.unwrap_or(0),
                author: login(c.user.as_ref()),
                body: c.body.unwrap_or_default(),
                created_at: format_time(c.created_at),
            }));
            if n < PAGE {
                break;
            }
            page += 1;
        }
        Ok(all)
    }

    async fn deps(&self, number: i64) -> Result<(Vec<RelIssue>, Vec<RelIssue>), FjiError> {
        let blocked = self
            .page_issues(|page| {
                self.api
                    .issue_list_issue_dependencies(self.owner(), self.name(), number)
                    .page(page)
                    .page_size(PAGE)
                    .send()
            })
            .await?;
        let blocks = self
            .page_issues(|page| {
                self.api
                    .issue_list_blocks(self.owner(), self.name(), number)
                    .page(page)
                    .page_size(PAGE)
                    .send()
            })
            .await?;
        Ok((
            blocked.into_iter().map(rel_issue).collect(),
            blocks.into_iter().map(rel_issue).collect(),
        ))
    }

    async fn page_issues<F, Fut>(&self, mut fetch: F) -> Result<Vec<ApiIssue>, FjiError>
    where
        F: FnMut(u32) -> Fut,
        Fut: std::future::Future<Output = Result<Vec<ApiIssue>, ForgejoError>>,
    {
        let mut page = 1u32;
        let mut all = Vec::new();
        loop {
            let items = fetch(page).await.map_err(map_forgejo)?;
            let n = items.len() as u32;
            all.extend(items);
            if n < PAGE {
                break;
            }
            page += 1;
        }
        Ok(all)
    }

    async fn ensure_labels_exist(&self, names: &[String]) -> Result<(), FjiError> {
        if names.is_empty() {
            return Ok(());
        }
        let _ = self.resolve_label_ids(names).await?;
        Ok(())
    }

    async fn resolve_label_ids(&self, names: &[String]) -> Result<Vec<i64>, FjiError> {
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let mut page = 1u32;
        let mut labels = Vec::new();
        loop {
            let (_, items) = self
                .api
                .issue_list_labels(
                    self.owner(),
                    self.name(),
                    IssueListLabelsQuery { sort: None },
                )
                .page(page)
                .page_size(PAGE)
                .send()
                .await
                .map_err(map_forgejo)?;
            let n = items.len() as u32;
            labels.extend(items);
            if n < PAGE {
                break;
            }
            page += 1;
        }
        let mut ids = Vec::new();
        for name in names {
            match labels
                .iter()
                .find(|l| l.name.as_deref() == Some(name.as_str()))
            {
                Some(label) => ids.push(label.id.unwrap_or(0)),
                None => {
                    return Err(FjiError::usage(
                        "unknown_label",
                        format!("label {name:?} does not exist on this repository"),
                    ));
                }
            }
        }
        Ok(ids)
    }
}

fn project_issue(
    issue: &ApiIssue,
    blocked_by: Option<Vec<RelIssue>>,
    blocks: Option<Vec<RelIssue>>,
    comments: Option<Vec<CommentView>>,
) -> IssueView {
    let state = match issue.state {
        Some(StateType::Closed) => "closed",
        _ => "open",
    };
    IssueView {
        number: issue.number.unwrap_or(0),
        url: issue_url(issue),
        title: issue.title.clone().unwrap_or_default(),
        body: issue.body.clone().unwrap_or_default(),
        state: state.into(),
        labels: issue
            .labels
            .as_ref()
            .map(|ls| ls.iter().filter_map(|l| l.name.clone()).collect())
            .unwrap_or_default(),
        assignees: current_assignees(issue),
        author: login(issue.user.as_ref()),
        blocked_by,
        blocks,
        comments,
        created_at: format_time(issue.created_at),
        updated_at: format_time(issue.updated_at),
    }
}

fn rel_issue(issue: ApiIssue) -> RelIssue {
    let state = match issue.state {
        Some(StateType::Closed) => "closed",
        _ => "open",
    };
    RelIssue {
        number: issue.number.unwrap_or(0),
        state: state.into(),
        url: issue_url(&issue),
        title: issue.title.unwrap_or_default(),
    }
}

fn issue_url(issue: &ApiIssue) -> String {
    issue
        .html_url
        .as_ref()
        .map(|u| u.to_string())
        .unwrap_or_default()
}

fn login(user: Option<&forgejo_api::structs::User>) -> String {
    user.and_then(|u| u.login.clone()).unwrap_or_default()
}

fn current_assignees(issue: &ApiIssue) -> Vec<String> {
    issue
        .assignees
        .as_ref()
        .map(|users| users.iter().filter_map(|u| u.login.clone()).collect())
        .unwrap_or_default()
}

fn is_uncertain(err: &ForgejoError) -> bool {
    matches!(err, ForgejoError::ReqwestError(e) if e.is_timeout() || e.is_request() && !e.is_connect())
}

fn http_of(err: &FjiError) -> Option<u16> {
    match err {
        FjiError::NotFound { http, .. }
        | FjiError::Conflict { http, .. }
        | FjiError::Http { http, .. } => *http,
        FjiError::Blocked { .. } => Some(412),
        _ => None,
    }
}

pub fn map_forgejo(err: ForgejoError) -> FjiError {
    match err {
        ForgejoError::ApiError(api) => {
            let message = api.message().unwrap_or("forgejo api error").to_string();
            if message.contains("open dependencies") {
                return FjiError::Blocked {
                    error: message,
                    blocked_by: None,
                };
            }
            match api.error_kind() {
                ApiErrorKind::NotFound { .. } => FjiError::NotFound {
                    error: message,
                    http: Some(404),
                },
                ApiErrorKind::Other(status) if status.as_u16() == 412 => FjiError::Blocked {
                    error: message,
                    blocked_by: None,
                },
                ApiErrorKind::Other(status) if status.as_u16() == 409 => FjiError::Conflict {
                    error: message,
                    http: Some(409),
                },
                ApiErrorKind::Other(status) => FjiError::Http {
                    code: "http",
                    error: message,
                    http: Some(status.as_u16()),
                },
                _ => FjiError::Http {
                    code: "http",
                    error: message,
                    http: None,
                },
            }
        }
        ForgejoError::UnexpectedStatusCode(status) if status.as_u16() == 412 => FjiError::Blocked {
            error: "cannot close this issue because it still has open dependencies".into(),
            blocked_by: None,
        },
        ForgejoError::UnexpectedStatusCode(status) if status.as_u16() == 409 => {
            FjiError::Conflict {
                error: status.to_string(),
                http: Some(409),
            }
        }
        ForgejoError::UnexpectedStatusCode(status) if status.as_u16() == 404 => {
            FjiError::NotFound {
                error: status.to_string(),
                http: Some(404),
            }
        }
        ForgejoError::UnexpectedStatusCode(status) => FjiError::Http {
            code: "http",
            error: status.to_string(),
            http: Some(status.as_u16()),
        },
        ForgejoError::ReqwestError(e) if e.is_timeout() => FjiError::Uncertain {
            error: e.to_string(),
            issue_number: None,
            issue_url: None,
        },
        ForgejoError::ReqwestError(e) => FjiError::Http {
            code: if e.is_connect() { "network" } else { "http" },
            error: e.to_string(),
            http: e.status().map(|s| s.as_u16()),
        },
        other => FjiError::Http {
            code: "http",
            error: other.to_string(),
            http: None,
        },
    }
}
