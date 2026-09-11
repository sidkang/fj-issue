use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::config::IssueRef;
use crate::error::FjiError;

#[derive(Parser, Debug)]
#[command(
    name = "fji",
    version,
    about = "Agent-first Forgejo issue CLI",
    disable_help_subcommand = true
)]
pub struct Cli {
    #[arg(short = 'H', long = "host", global = true)]
    pub host: Option<String>,
    #[arg(short = 'R', long = "repo", global = true)]
    pub repo: Option<String>,
    #[arg(short = 'C', long = "cwd", global = true)]
    pub cwd: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Create {
        #[arg(long)]
        title: String,
        #[arg(long)]
        body: Option<String>,
        #[arg(long = "body-file")]
        body_file: Option<PathBuf>,
        #[arg(long = "label")]
        labels: Vec<String>,
        #[arg(long = "assignee")]
        assignees: Vec<String>,
        #[arg(long = "blocked-by")]
        blocked_by: Vec<String>,
    },
    View {
        issue: String,
    },
    List {
        #[arg(long, default_value = "open")]
        state: String,
        #[arg(long = "label")]
        labels: Vec<String>,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long)]
        search: Option<String>,
        #[arg(long)]
        limit: Option<u32>,
        #[arg(long)]
        all: bool,
        #[arg(long = "no-deps")]
        no_deps: bool,
    },
    Comment {
        issue: String,
        #[arg(long)]
        body: Option<String>,
        #[arg(long = "body-file")]
        body_file: Option<PathBuf>,
    },
    Edit {
        issue: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        body: Option<String>,
        #[arg(long = "body-file")]
        body_file: Option<PathBuf>,
    },
    Label {
        #[command(subcommand)]
        action: LabelAction,
    },
    Assign {
        issue: String,
        users: Vec<String>,
    },
    Unassign {
        issue: String,
        users: Vec<String>,
    },
    Close {
        issue: String,
    },
    Reopen {
        issue: String,
    },
    Dep {
        #[command(subcommand)]
        action: DepAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum LabelAction {
    Add { issue: String, labels: Vec<String> },
    Rm { issue: String, labels: Vec<String> },
}

#[derive(Subcommand, Debug)]
pub enum DepAction {
    Add {
        issue: String,
        #[arg(long = "blocked-by", required = true)]
        blocked_by: Vec<String>,
    },
    Rm {
        issue: String,
        #[arg(long = "blocked-by", required = true)]
        blocked_by: String,
    },
    List {
        issue: String,
    },
}

#[derive(Debug, Clone)]
pub enum Command {
    Create {
        title: String,
        body: String,
        labels: Vec<String>,
        assignees: Vec<String>,
        blocked_by: Vec<IssueRef>,
    },
    View {
        issue: IssueRef,
    },
    List {
        state: ListState,
        labels: Vec<String>,
        assignee: Option<String>,
        search: Option<String>,
        limit: u32,
        all: bool,
        no_deps: bool,
    },
    Comment {
        issue: IssueRef,
        body: String,
    },
    Edit {
        issue: IssueRef,
        title: Option<String>,
        body: Option<String>,
    },
    LabelAdd {
        issue: IssueRef,
        labels: Vec<String>,
    },
    LabelRm {
        issue: IssueRef,
        labels: Vec<String>,
    },
    Assign {
        issue: IssueRef,
        users: Vec<String>,
    },
    Unassign {
        issue: IssueRef,
        users: Vec<String>,
    },
    Close {
        issue: IssueRef,
    },
    Reopen {
        issue: IssueRef,
    },
    DepAdd {
        issue: IssueRef,
        blocked_by: Vec<IssueRef>,
    },
    DepRm {
        issue: IssueRef,
        blocked_by: IssueRef,
    },
    DepList {
        issue: IssueRef,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListState {
    Open,
    Closed,
    All,
}

impl Commands {
    pub fn into_command(self) -> Result<Command, FjiError> {
        Ok(match self {
            Commands::Create {
                title,
                body,
                body_file,
                labels,
                assignees,
                blocked_by,
            } => {
                let body = read_body(body, body_file)?;
                if title.trim().is_empty() {
                    return Err(FjiError::usage("usage", "--title must not be empty"));
                }
                Command::Create {
                    title,
                    body,
                    labels,
                    assignees,
                    blocked_by: parse_refs(&blocked_by)?,
                }
            }
            Commands::View { issue } => Command::View {
                issue: IssueRef::parse(&issue)?,
            },
            Commands::List {
                state,
                labels,
                assignee,
                search,
                limit,
                all,
                no_deps,
            } => {
                if all && limit.is_some() {
                    return Err(FjiError::usage(
                        "usage",
                        "--all and --limit cannot be combined",
                    ));
                }
                let limit = limit.unwrap_or(30);
                if limit < 1 {
                    return Err(FjiError::usage("usage", "--limit must be >= 1"));
                }
                if limit == u32::MAX {
                    return Err(FjiError::usage("usage", "--limit is too large"));
                }
                let state = match state.as_str() {
                    "open" => ListState::Open,
                    "closed" => ListState::Closed,
                    "all" => ListState::All,
                    other => {
                        return Err(FjiError::usage(
                            "usage",
                            format!("--state must be open|closed|all, got {other:?}"),
                        ));
                    }
                };
                Command::List {
                    state,
                    labels,
                    assignee,
                    search,
                    limit,
                    all,
                    no_deps,
                }
            }
            Commands::Comment {
                issue,
                body,
                body_file,
            } => Command::Comment {
                issue: IssueRef::parse(&issue)?,
                body: read_body(body, body_file)?,
            },
            Commands::Edit {
                issue,
                title,
                body,
                body_file,
            } => {
                if body.is_some() && body_file.is_some() {
                    return Err(FjiError::usage("usage", "--body and --body-file conflict"));
                }
                let body = match (body, body_file) {
                    (None, None) => None,
                    (Some(b), None) => Some(b),
                    (None, Some(path)) => Some(read_body_file(&path)?),
                    (Some(_), Some(_)) => unreachable!(),
                };
                if title.is_none() && body.is_none() {
                    return Err(FjiError::usage(
                        "usage",
                        "edit requires --title and/or --body",
                    ));
                }
                Command::Edit {
                    issue: IssueRef::parse(&issue)?,
                    title,
                    body,
                }
            }
            Commands::Label { action } => match action {
                LabelAction::Add { issue, labels } => {
                    if labels.is_empty() {
                        return Err(FjiError::usage(
                            "usage",
                            "label add requires at least one label",
                        ));
                    }
                    Command::LabelAdd {
                        issue: IssueRef::parse(&issue)?,
                        labels,
                    }
                }
                LabelAction::Rm { issue, labels } => {
                    if labels.is_empty() {
                        return Err(FjiError::usage(
                            "usage",
                            "label rm requires at least one label",
                        ));
                    }
                    Command::LabelRm {
                        issue: IssueRef::parse(&issue)?,
                        labels,
                    }
                }
            },
            Commands::Assign { issue, users } => {
                if users.is_empty() {
                    return Err(FjiError::usage(
                        "usage",
                        "assign requires at least one user",
                    ));
                }
                Command::Assign {
                    issue: IssueRef::parse(&issue)?,
                    users,
                }
            }
            Commands::Unassign { issue, users } => {
                if users.is_empty() {
                    return Err(FjiError::usage(
                        "usage",
                        "unassign requires at least one user",
                    ));
                }
                Command::Unassign {
                    issue: IssueRef::parse(&issue)?,
                    users,
                }
            }
            Commands::Close { issue } => Command::Close {
                issue: IssueRef::parse(&issue)?,
            },
            Commands::Reopen { issue } => Command::Reopen {
                issue: IssueRef::parse(&issue)?,
            },
            Commands::Dep { action } => match action {
                DepAction::Add { issue, blocked_by } => Command::DepAdd {
                    issue: IssueRef::parse(&issue)?,
                    blocked_by: parse_refs(&blocked_by)?,
                },
                DepAction::Rm { issue, blocked_by } => Command::DepRm {
                    issue: IssueRef::parse(&issue)?,
                    blocked_by: IssueRef::parse(&blocked_by)?,
                },
                DepAction::List { issue } => Command::DepList {
                    issue: IssueRef::parse(&issue)?,
                },
            },
        })
    }
}

fn parse_refs(values: &[String]) -> Result<Vec<IssueRef>, FjiError> {
    values.iter().map(|s| IssueRef::parse(s)).collect()
}

fn read_body(body: Option<String>, body_file: Option<PathBuf>) -> Result<String, FjiError> {
    match (body, body_file) {
        (Some(_), Some(_)) => Err(FjiError::usage("usage", "--body and --body-file conflict")),
        (None, None) => Err(FjiError::usage("usage", "provide --body or --body-file")),
        (Some(body), None) => Ok(body),
        (None, Some(path)) => read_body_file(&path),
    }
}

fn read_body_file(path: &PathBuf) -> Result<String, FjiError> {
    if path.as_os_str() == "-" {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
            .map_err(|e| FjiError::usage("usage", format!("failed to read stdin: {e}")))?;
        if buf.contains('\0') {
            return Err(FjiError::usage("usage", "stdin body must be text"));
        }
        return Ok(buf);
    }
    let bytes = std::fs::read(path)
        .map_err(|e| FjiError::usage("usage", format!("failed to read {}: {e}", path.display())))?;
    if bytes.contains(&0) {
        return Err(FjiError::usage("usage", "body file must be text"));
    }
    String::from_utf8(bytes).map_err(|_| FjiError::usage("usage", "body file must be UTF-8 text"))
}
