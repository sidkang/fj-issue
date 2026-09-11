pub mod cli;
pub mod client;
pub mod config;
pub mod error;
pub mod git;
pub mod model;

use cli::Command;
use client::Client;
use config::{Context, IssueRef, resolve};
use error::FjiError;

pub async fn execute(cmd: Command, ctx: Context) -> Result<serde_json::Value, FjiError> {
    let (issue, extras) = cmd.refs();
    let resolved = resolve(&ctx, issue.as_ref(), &extras)?;
    let client = Client::connect(&resolved, ctx.timeout)?;
    let value = match cmd {
        Command::Create {
            title,
            body,
            labels,
            assignees,
            blocked_by,
        } => {
            let issue = client
                .create(title, body, labels, assignees, blocked_by)
                .await?;
            serde_json::to_value(issue).expect("issue json")
        }
        Command::View { issue } => {
            let issue = client.view(issue.number).await?;
            serde_json::to_value(issue).expect("issue json")
        }
        Command::List {
            state,
            labels,
            assignee,
            search,
            limit,
            all,
            no_deps,
        } => {
            let list = client
                .list(state, labels, assignee, search, limit, all, no_deps)
                .await?;
            serde_json::to_value(list).expect("list json")
        }
        Command::Comment { issue, body } => {
            let comment = client.comment(issue.number, body).await?;
            serde_json::to_value(comment).expect("comment json")
        }
        Command::Edit { issue, title, body } => {
            let issue = client.edit(issue.number, title, body).await?;
            serde_json::to_value(issue).expect("issue json")
        }
        Command::LabelAdd { issue, labels } => {
            let issue = client.label_add(issue.number, labels).await?;
            serde_json::to_value(issue).expect("issue json")
        }
        Command::LabelRm { issue, labels } => {
            let issue = client.label_rm(issue.number, labels).await?;
            serde_json::to_value(issue).expect("issue json")
        }
        Command::Assign { issue, users } => {
            let issue = client.assign(issue.number, users).await?;
            serde_json::to_value(issue).expect("issue json")
        }
        Command::Unassign { issue, users } => {
            let issue = client.unassign(issue.number, users).await?;
            serde_json::to_value(issue).expect("issue json")
        }
        Command::Close { issue } => {
            let issue = client.close(issue.number).await?;
            serde_json::to_value(issue).expect("issue json")
        }
        Command::Reopen { issue } => {
            let issue = client.reopen(issue.number).await?;
            serde_json::to_value(issue).expect("issue json")
        }
        Command::DepAdd { issue, blocked_by } => {
            let issue = client.dep_add(issue.number, blocked_by).await?;
            serde_json::to_value(issue).expect("issue json")
        }
        Command::DepRm { issue, blocked_by } => {
            let issue = client.dep_rm(issue.number, blocked_by).await?;
            serde_json::to_value(issue).expect("issue json")
        }
        Command::DepList { issue } => {
            let issue = client.dep_list(issue.number).await?;
            serde_json::to_value(issue).expect("issue json")
        }
    };
    Ok(value)
}

impl Command {
    fn refs(&self) -> (Option<IssueRef>, Vec<IssueRef>) {
        match self {
            Command::Create { blocked_by, .. } => (None, blocked_by.clone()),
            Command::View { issue }
            | Command::Comment { issue, .. }
            | Command::Edit { issue, .. }
            | Command::LabelAdd { issue, .. }
            | Command::LabelRm { issue, .. }
            | Command::Assign { issue, .. }
            | Command::Unassign { issue, .. }
            | Command::Close { issue }
            | Command::Reopen { issue }
            | Command::DepList { issue } => (Some(issue.clone()), Vec::new()),
            Command::DepAdd { issue, blocked_by } => (Some(issue.clone()), blocked_by.clone()),
            Command::DepRm { issue, blocked_by } => (Some(issue.clone()), vec![blocked_by.clone()]),
            Command::List { .. } => (None, Vec::new()),
        }
    }
}
