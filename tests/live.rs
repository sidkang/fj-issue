//! Authorized live checks. Run with:
//! `FJI_LIVE=1 FJI_HOST=git.882816.xyz FJI_REPO=sid/hello-world FJI_TOKEN=... cargo test --test live -- --nocapture`

use fj_issue::cli::Command;
use fj_issue::config::{Context, IssueRef};
use fj_issue::execute;

fn live_ctx() -> Option<Context> {
    if std::env::var("FJI_LIVE").ok().as_deref() != Some("1") {
        return None;
    }
    let token = std::env::var("FJI_TOKEN")
        .ok()
        .or_else(|| std::env::var("FORGEJO_TOKEN").ok())?;
    Some(Context {
        host: Some(std::env::var("FJI_HOST").unwrap_or_else(|_| "git.882816.xyz".into())),
        repo: Some(std::env::var("FJI_REPO").unwrap_or_else(|_| "sid/hello-world".into())),
        token: Some(token),
        ..Context::default()
    })
}

fn frontier(items: &[serde_json::Value]) -> Vec<i64> {
    items
        .iter()
        .filter(|item| item["state"] == "open")
        .filter(|item| {
            item["blocked_by"]
                .as_array()
                .map(|deps| deps.iter().all(|dep| dep["state"] != "open"))
                .unwrap_or(false)
        })
        .filter_map(|item| item["number"].as_i64())
        .collect()
}

#[tokio::test(flavor = "current_thread")]
async fn live_frontier_from_dependencies() {
    let Some(ctx) = live_ctx() else {
        return;
    };
    let blocker = execute(
        Command::Create {
            title: "[fji-live] blocker".into(),
            body: "blocks a child".into(),
            labels: vec![],
            assignees: vec![],
            blocked_by: vec![],
        },
        ctx.clone(),
    )
    .await
    .unwrap();
    let blocker_n = blocker["number"].as_i64().unwrap();
    let child = execute(
        Command::Create {
            title: "[fji-live] child".into(),
            body: "blocked".into(),
            labels: vec![],
            assignees: vec![],
            blocked_by: vec![IssueRef {
                repo: None,
                number: blocker_n,
            }],
        },
        ctx.clone(),
    )
    .await
    .unwrap();
    let child_n = child["number"].as_i64().unwrap();
    let free = execute(
        Command::Create {
            title: "[fji-live] free".into(),
            body: "unblocked".into(),
            labels: vec![],
            assignees: vec![],
            blocked_by: vec![],
        },
        ctx.clone(),
    )
    .await
    .unwrap();
    let free_n = free["number"].as_i64().unwrap();

    let listed = execute(
        Command::List {
            state: fj_issue::cli::ListState::Open,
            labels: vec![],
            assignee: None,
            search: None,
            limit: 50,
            all: false,
            no_deps: false,
        },
        ctx.clone(),
    )
    .await
    .unwrap();
    let items = listed["items"].as_array().cloned().unwrap_or_default();
    let ours: Vec<_> = items
        .into_iter()
        .filter(|i| {
            let n = i["number"].as_i64().unwrap_or(0);
            n == blocker_n || n == child_n || n == free_n
        })
        .collect();
    let edge = frontier(&ours);
    assert!(
        edge.contains(&free_n) && edge.contains(&blocker_n) && !edge.contains(&child_n),
        "frontier={edge:?} child={child_n} free={free_n} blocker={blocker_n}"
    );

    execute(
        Command::Close {
            issue: IssueRef {
                repo: None,
                number: blocker_n,
            },
        },
        ctx.clone(),
    )
    .await
    .unwrap();
    let listed = execute(
        Command::List {
            state: fj_issue::cli::ListState::Open,
            labels: vec![],
            assignee: None,
            search: None,
            limit: 50,
            all: false,
            no_deps: false,
        },
        ctx.clone(),
    )
    .await
    .unwrap();
    let ours: Vec<_> = listed["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| {
            let n = i["number"].as_i64().unwrap_or(0);
            n == child_n || n == free_n
        })
        .collect();
    let edge = frontier(&ours);
    assert!(
        edge.contains(&child_n) && edge.contains(&free_n),
        "after close frontier={edge:?}"
    );

    execute(
        Command::Close {
            issue: IssueRef {
                repo: None,
                number: child_n,
            },
        },
        ctx.clone(),
    )
    .await
    .unwrap();
    execute(
        Command::Close {
            issue: IssueRef {
                repo: None,
                number: free_n,
            },
        },
        ctx,
    )
    .await
    .unwrap();
}
