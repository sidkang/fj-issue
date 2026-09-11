use fj_issue::cli::Command;
use fj_issue::config::{Context, IssueRef};
use fj_issue::error::FjiError;
use fj_issue::execute;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx(server: &MockServer) -> Context {
    Context {
        host: Some(server.uri()),
        repo: Some("sid/hello-world".into()),
        token: Some("tok".into()),
        ..Context::default()
    }
}

fn issue_body(number: i64, title: &str) -> Value {
    json!({
        "number": number,
        "title": title,
        "body": "body",
        "state": "open",
        "html_url": format!("https://git.example/sid/hello-world/issues/{number}"),
        "user": {"login": "sid"},
        "labels": [],
        "assignees": [],
        "created_at": "2026-09-11T15:46:31Z",
        "updated_at": "2026-09-11T15:46:31Z"
    })
}

async fn mount_issue_reads(server: &MockServer, number: i64, title: &str) {
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/repos/sid/hello-world/issues/{number}"
        )))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-total-count", "1")
                .set_body_json(issue_body(number, title)),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/repos/sid/hello-world/issues/{number}/comments"
        )))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-total-count", "0")
                .set_body_json(json!([])),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/repos/sid/hello-world/issues/{number}/dependencies"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/repos/sid/hello-world/issues/{number}/blocks"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn view_issue() {
    let server = MockServer::start().await;
    mount_issue_reads(&server, 1, "map").await;
    let value = execute(
        Command::View {
            issue: IssueRef::parse("1").unwrap(),
        },
        ctx(&server),
    )
    .await
    .unwrap();
    assert_eq!(value["number"], 1);
    assert_eq!(value["title"], "map");
    assert_eq!(value["author"], "sid");
    assert!(value.get("milestone").is_none());
    assert!(value.get("blocked_by").is_some());
    assert!(value["comments"].is_array());
    let dump = value.to_string();
    assert!(!dump.contains("email"));
}

#[tokio::test(flavor = "current_thread")]
async fn unknown_label_does_not_create() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/labels"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-total-count", "0")
                .set_body_json(json!([])),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/sid/hello-world/issues"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let err = execute(
        Command::Create {
            title: "t".into(),
            body: "b".into(),
            labels: vec!["missing".into()],
            assignees: vec![],
            blocked_by: vec![],
        },
        ctx(&server),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        FjiError::Usage {
            code: "unknown_label",
            ..
        }
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn create_then_partial_dep() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/sid/hello-world/issues"))
        .respond_with(ResponseTemplate::new(201).set_body_json(issue_body(7, "child")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/sid/hello-world/issues/7/dependencies"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(json!({"message": "not found", "errors": []})),
        )
        .mount(&server)
        .await;
    let err = execute(
        Command::Create {
            title: "child".into(),
            body: "b".into(),
            labels: vec![],
            assignees: vec![],
            blocked_by: vec![IssueRef::parse("99").unwrap()],
        },
        ctx(&server),
    )
    .await
    .unwrap_err();
    match err {
        FjiError::PartialCreate {
            issue_number,
            applied,
            failed,
            ..
        } => {
            assert_eq!(issue_number, 7);
            assert!(applied.is_empty());
            assert_eq!(failed.number, 99);
        }
        other => panic!("expected partial_create, got {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn close_blocked() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/api/v1/repos/sid/hello-world/issues/5"))
        .respond_with(ResponseTemplate::new(412).set_body_json(json!({
            "message": "cannot close this issue because it still has open dependencies"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/issues/5/dependencies"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([issue_body(4, "blocker")])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/issues/5/blocks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    let err = execute(
        Command::Close {
            issue: IssueRef::parse("5").unwrap(),
        },
        ctx(&server),
    )
    .await
    .unwrap_err();
    assert_eq!(err.exit_code(), 4);
    match err {
        FjiError::Blocked { blocked_by, .. } => {
            assert_eq!(blocked_by, Some(vec![4]));
        }
        other => panic!("expected blocked, got {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn list_no_deps_omits_fields() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/issues"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-total-count", "1")
                .set_body_json(json!([issue_body(1, "map")])),
        )
        .mount(&server)
        .await;
    let value = execute(
        Command::List {
            state: fj_issue::cli::ListState::Open,
            labels: vec![],
            assignee: None,
            search: None,
            limit: 30,
            all: false,
            no_deps: true,
        },
        ctx(&server),
    )
    .await
    .unwrap();
    assert_eq!(value["truncated"], false);
    assert!(value["items"][0].get("blocked_by").is_none());
    assert!(value["items"][0].get("comments").is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn list_truncated_without_trusting_missing_count() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/issues"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([issue_body(1, "a"), issue_body(2, "b")])),
        )
        .mount(&server)
        .await;
    let value = execute(
        Command::List {
            state: fj_issue::cli::ListState::Open,
            labels: vec![],
            assignee: None,
            search: None,
            limit: 1,
            all: false,
            no_deps: true,
        },
        ctx(&server),
    )
    .await
    .unwrap();
    assert_eq!(value["truncated"], true);
    assert_eq!(value["items"].as_array().unwrap().len(), 1);
    assert!(value.get("total_count").is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn list_enrichment_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/issues"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-total-count", "1")
                .set_body_json(json!([issue_body(1, "map")])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/issues/1/dependencies"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let err = execute(
        Command::List {
            state: fj_issue::cli::ListState::Open,
            labels: vec![],
            assignee: None,
            search: None,
            limit: 30,
            all: false,
            no_deps: false,
        },
        ctx(&server),
    )
    .await
    .unwrap_err();
    assert_eq!(err.exit_code(), 2);
}
