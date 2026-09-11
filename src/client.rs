use forgejo_api::structs::{
    CreateIssueCommentOption, CreateIssueOption, DeleteLabelsOption, EditIssueOption,
    Issue as ApiIssue, IssueGetCommentsQuery, IssueLabelsOption, IssueListIssuesQuery,
    IssueListIssuesQueryState, IssueListIssuesQueryType, IssueListLabelsQuery, IssueMeta,
    StateType,
};
use std::future::Future;
use std::time::Duration;

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
    timeout: Duration,
}

impl Client {
    pub fn connect(resolved: &Resolved, timeout: Duration) -> Result<Self, FjiError> {
        let api =
            Forgejo::with_user_agent(Auth::Token(&resolved.token), resolved.host_url.clone(), UA)
                .map_err(map_read)?;
        Ok(Self {
            api,
            repo: resolved.repo.clone(),
            timeout,
        })
    }

    async fn read<T>(
        &self,
        fut: impl Future<Output = Result<T, ForgejoError>>,
    ) -> Result<T, FjiError> {
        timed_read(fut, self.timeout).await
    }

    async fn write<T>(
        &self,
        fut: impl Future<Output = Result<T, ForgejoError>>,
        issue_number: Option<i64>,
        issue_url: Option<String>,
    ) -> Result<T, FjiError> {
        timed_write(fut, self.timeout, issue_number, issue_url).await
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
            .write(
                self.api
                    .issue_create_comment(
                        self.owner(),
                        self.name(),
                        number,
                        CreateIssueCommentOption {
                            body,
                            updated_at: None,
                        },
                    )
                    .send(),
                Some(number),
                None,
            )
            .await?;
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
            .write(
                self.api
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
                    .send(),
                Some(number),
                None,
            )
            .await?;
        self.attach_deps(issue).await
    }

    pub async fn label_add(&self, number: i64, labels: Vec<String>) -> Result<IssueView, FjiError> {
        self.ensure_labels_exist(&labels).await?;
        let values = labels.into_iter().map(Value::String).collect::<Vec<_>>();
        self.write(
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
                .send(),
            Some(number),
            None,
        )
        .await?;
        self.view_without_comments(number).await
    }

    pub async fn label_rm(&self, number: i64, labels: Vec<String>) -> Result<IssueView, FjiError> {
        self.ensure_labels_exist(&labels).await?;
        let issue = self.get_issue(number).await?;
        let remaining: Vec<String> = issue
            .labels
            .as_ref()
            .map(|ls| {
                ls.iter()
                    .filter_map(|l| l.name.clone())
                    .filter(|name| !labels.iter().any(|rm| rm == name))
                    .collect()
            })
            .unwrap_or_default();
        if remaining.is_empty() {
            self.write(
                self.api
                    .issue_clear_labels(
                        self.owner(),
                        self.name(),
                        number,
                        DeleteLabelsOption { updated_at: None },
                    )
                    .send(),
                Some(number),
                None,
            )
            .await?;
        } else {
            let values = remaining.into_iter().map(Value::String).collect::<Vec<_>>();
            self.write(
                self.api
                    .issue_replace_labels(
                        self.owner(),
                        self.name(),
                        number,
                        IssueLabelsOption {
                            labels: Some(values),
                            updated_at: None,
                        },
                    )
                    .send(),
                Some(number),
                None,
            )
            .await?;
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
            .write(
                self.api
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
                    .send(),
                Some(number),
                None,
            )
            .await?;
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
            .write(
                self.api
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
                    .send(),
                Some(number),
                None,
            )
            .await;
        match result {
            Ok(issue) => self.attach_deps(issue).await,
            Err(mapped) => {
                if state == "closed"
                    && let Ok(current) = self.get_issue(number).await
                    && current.state != Some(StateType::Closed)
                    && let Ok((blocked, _)) = self.deps(number).await
                {
                    let open: Vec<i64> = blocked
                        .iter()
                        .filter(|b| b.state == "open")
                        .map(|b| b.number)
                        .collect();
                    if !open.is_empty() {
                        return Err(FjiError::Blocked {
                            error: "cannot close this issue because it still has open dependencies"
                                .into(),
                            blocked_by: Some(open),
                        });
                    }
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
            .write(
                self.api
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
                    .send(),
                None,
                None,
            )
            .await?;
        let number = issue.number.ok_or_else(|| FjiError::Http {
            code: "http",
            error: "created issue has no number".into(),
            http: None,
        })?;
        let url = issue_url(&issue);
        let preserve = |err: FjiError| FjiError::Uncertain {
            error: err.to_string(),
            issue_number: Some(number),
            issue_url: Some(url.clone()),
        };
        if blocked_by.is_empty() {
            return self.attach_deps(issue).await.map_err(preserve);
        }
        match self.add_deps_inner(number, &url, &blocked_by).await {
            Ok(()) => self.attach_deps(issue).await.map_err(preserve),
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

    pub async fn dep_rm(&self, number: i64, blocked_by: IssueRef) -> Result<IssueView, FjiError> {
        self.write(
            self.api
                .issue_remove_issue_dependencies(
                    self.owner(),
                    self.name(),
                    number,
                    IssueMeta {
                        index: Some(blocked_by.number),
                        owner: Some(self.owner().into()),
                        repo: Some(self.name().into()),
                    },
                )
                .send(),
            Some(number),
            None,
        )
        .await?;
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
        let searching = search.is_some();
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

        let (items, truncated, mut total_count) = if all {
            let (items, total) = self.list_all(query).await?;
            (items, false, total)
        } else {
            self.list_page(query, limit).await?
        };
        if searching {
            total_count = None;
        }

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
        let want = (limit as usize).saturating_add(1);
        let (items, total) = self.collect_issues(query, Some(want)).await?;
        let truncated =
            items.len() > limit as usize || total.map(|t| t > i64::from(limit)).unwrap_or(false);
        let mut items = items;
        items.truncate(limit as usize);
        Ok((items, truncated, total))
    }

    async fn list_all(
        &self,
        query: IssueListIssuesQuery,
    ) -> Result<(Vec<ApiIssue>, Option<i64>), FjiError> {
        self.collect_issues(query, None).await
    }

    async fn collect_issues(
        &self,
        query: IssueListIssuesQuery,
        want: Option<usize>,
    ) -> Result<(Vec<ApiIssue>, Option<i64>), FjiError> {
        let mut page = 1u32;
        let mut page_size = PAGE;
        let mut all = Vec::new();
        let mut total = None;
        loop {
            let (headers, items) = self
                .read(
                    self.api
                        .issue_list_issues(self.owner(), self.name(), query.clone())
                        .page(page)
                        .page_size(page_size)
                        .send(),
                )
                .await?;
            if let Some(count) = headers.x_total_count {
                total = Some(count);
            }
            let n = items.len() as u32;
            all.extend(items);
            let have_all = total.map(|t| all.len() as i64 >= t).unwrap_or(false);
            if have_all || n == 0 || want.is_some_and(|w| all.len() >= w) {
                break;
            }
            if n < page_size {
                if total.map(|t| (all.len() as i64) < t).unwrap_or(false) {
                    page_size = n.max(1);
                    page += 1;
                    continue;
                }
                break;
            }
            page += 1;
        }
        Ok((all, total))
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
                .write(
                    self.api
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
                        .send(),
                    Some(number),
                    Some(url.to_string()),
                )
                .await;
            match result {
                Ok(_) => applied.push(target.number),
                Err(err @ FjiError::Uncertain { .. }) => return Err(err),
                Err(mapped) => {
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
        self.read(
            self.api
                .issue_get_issue(self.owner(), self.name(), number)
                .send(),
        )
        .await
    }

    async fn comments(&self, number: i64) -> Result<Vec<CommentView>, FjiError> {
        let mut page = 1u32;
        let mut page_size = PAGE;
        let mut all = Vec::new();
        let mut total = None;
        loop {
            let (headers, items) = self
                .read(
                    self.api
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
                        .page_size(page_size)
                        .send(),
                )
                .await?;
            if let Some(count) = headers.x_total_count {
                total = Some(count);
            }
            let n = items.len() as u32;
            all.extend(items.into_iter().map(|c| CommentView {
                id: c.id.unwrap_or(0),
                author: login(c.user.as_ref()),
                body: c.body.unwrap_or_default(),
                created_at: format_time(c.created_at),
            }));
            let have_all = total.map(|t| all.len() as i64 >= t).unwrap_or(false);
            if have_all || n == 0 {
                break;
            }
            if n < page_size {
                if total.map(|t| (all.len() as i64) < t).unwrap_or(false) {
                    page_size = n.max(1);
                    page += 1;
                    continue;
                }
                break;
            }
            page += 1;
        }
        Ok(all)
    }

    async fn deps(&self, number: i64) -> Result<(Vec<RelIssue>, Vec<RelIssue>), FjiError> {
        let blocked = self
            .page_issues(|page, page_size| {
                self.api
                    .issue_list_issue_dependencies(self.owner(), self.name(), number)
                    .page(page)
                    .page_size(page_size)
                    .send()
            })
            .await?;
        let blocks = self
            .page_issues(|page, page_size| {
                self.api
                    .issue_list_blocks(self.owner(), self.name(), number)
                    .page(page)
                    .page_size(page_size)
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
        F: FnMut(u32, u32) -> Fut,
        Fut: std::future::Future<Output = Result<Vec<ApiIssue>, ForgejoError>>,
    {
        let mut page = 1u32;
        let mut all = Vec::new();
        loop {
            let items = self.read(fetch(page, PAGE)).await?;
            let n = items.len() as u32;
            all.extend(items);
            if n == 0 || n < PAGE {
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
        let mut page_size = PAGE;
        let mut labels = Vec::new();
        let mut total = None;
        loop {
            let (headers, items) = self
                .read(
                    self.api
                        .issue_list_labels(
                            self.owner(),
                            self.name(),
                            IssueListLabelsQuery { sort: None },
                        )
                        .page(page)
                        .page_size(page_size)
                        .send(),
                )
                .await?;
            if let Some(count) = headers.x_total_count {
                total = Some(count);
            }
            let n = items.len() as u32;
            labels.extend(items);
            let have_all = total.map(|t| labels.len() as i64 >= t).unwrap_or(false);
            if have_all || n == 0 {
                break;
            }
            if n < page_size {
                if total.map(|t| (labels.len() as i64) < t).unwrap_or(false) {
                    page_size = n.max(1);
                    page += 1;
                    continue;
                }
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
    match err {
        ForgejoError::ReqwestError(e) => e.status().is_none() && !e.is_connect(),
        _ => false,
    }
}

async fn timed_read<T>(
    fut: impl Future<Output = Result<T, ForgejoError>>,
    timeout: Duration,
) -> Result<T, FjiError> {
    match tokio::time::timeout(timeout, fut).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(map_forgejo(err, false, None, None)),
        Err(_) => Err(FjiError::Http {
            code: "timeout",
            error: "request timed out".into(),
            http: None,
        }),
    }
}

async fn timed_write<T>(
    fut: impl Future<Output = Result<T, ForgejoError>>,
    timeout: Duration,
    issue_number: Option<i64>,
    issue_url: Option<String>,
) -> Result<T, FjiError> {
    match tokio::time::timeout(timeout, fut).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(map_forgejo(err, true, issue_number, issue_url)),
        Err(_) => Err(FjiError::Uncertain {
            error: "request timed out".into(),
            issue_number,
            issue_url,
        }),
    }
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

fn map_read(err: ForgejoError) -> FjiError {
    map_forgejo(err, false, None, None)
}

fn map_forgejo(
    err: ForgejoError,
    write: bool,
    issue_number: Option<i64>,
    issue_url: Option<String>,
) -> FjiError {
    if write && is_uncertain(&err) {
        return FjiError::Uncertain {
            error: err.to_string(),
            issue_number,
            issue_url,
        };
    }
    match err {
        ForgejoError::ApiError(api) => {
            let message = api.message().unwrap_or("forgejo api error").to_string();
            match api.error_kind() {
                ApiErrorKind::NotFound { .. } => FjiError::NotFound {
                    error: message,
                    http: Some(404),
                },
                ApiErrorKind::Forbidden => FjiError::Http {
                    code: "http",
                    error: message,
                    http: Some(403),
                },
                ApiErrorKind::Unauthorized => FjiError::Http {
                    code: "http",
                    error: message,
                    http: Some(401),
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
        ForgejoError::ReqwestError(e) => FjiError::Http {
            code: if e.is_connect() {
                "network"
            } else if e.is_timeout() {
                "timeout"
            } else {
                "http"
            },
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
