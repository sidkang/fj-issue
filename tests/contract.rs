use std::io::Read;
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use fj_issue::cli::Command;
use fj_issue::config::{Context, IssueRef};
use fj_issue::error::FjiError;
use fj_issue::execute;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx(server: &MockServer) -> Context {
    Context {
        host: Some(server.uri()),
        repo: Some("sid/hello-world".into()),
        token: Some("tok".into()),
        ..Context::default()
    }
}

fn issue_json(number: i64, title: &str) -> Value {
    json!({
        "number": number,
        "title": title,
        "body": "body",
        "state": "open",
        "html_url": format!("https://git.example/sid/hello-world/issues/{number}"),
        "user": {"login": "sid", "email": "hidden@example.com"},
        "labels": [],
        "assignees": [],
        "milestone": {"title": "ignore-me"},
        "created_at": "2026-09-11T15:46:31Z",
        "updated_at": "2026-09-11T15:46:31Z"
    })
}

fn comment_json(id: i64, body: &str) -> Value {
    json!({
        "id": id,
        "body": body,
        "user": {"login": "sid", "email": "hidden@example.com"},
        "created_at": "2026-09-11T15:46:31Z"
    })
}

async fn empty_deps(server: &MockServer, number: i64) {
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

fn spawn_drop_server() -> (String, thread::JoinHandle<usize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 16384];
        stream.read(&mut buf).unwrap_or(0)
    });
    (format!("http://{addr}"), handle)
}

#[tokio::test(flavor = "current_thread")]
async fn dropped_create_response_is_uncertain() {
    let (host, handle) = spawn_drop_server();
    let ctx = Context {
        host: Some(host),
        repo: Some("sid/hello-world".into()),
        token: Some("tok".into()),
        timeout: Duration::from_millis(800),
        ..Context::default()
    };
    let err = execute(
        Command::Create {
            title: "t".into(),
            body: "b".into(),
            labels: vec![],
            assignees: vec![],
            blocked_by: vec![],
        },
        ctx,
    )
    .await
    .unwrap_err();
    let _ = handle.join();
    assert!(
        matches!(
            err,
            FjiError::Uncertain {
                issue_number: None,
                ..
            }
        ),
        "got {err:?}"
    );
    assert_eq!(err.exit_code(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn view_status_matrix() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/issues/404"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "missing"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/issues/403"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"message": "nope"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/issues/500"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let not_found = execute(
        Command::View {
            issue: IssueRef::parse("404").unwrap(),
        },
        ctx(&server),
    )
    .await
    .unwrap_err();
    assert_eq!(not_found.exit_code(), 3);
    let forbidden = execute(
        Command::View {
            issue: IssueRef::parse("403").unwrap(),
        },
        ctx(&server),
    )
    .await
    .unwrap_err();
    assert_eq!(forbidden.exit_code(), 2);
    let json = forbidden.to_json();
    assert_eq!(json["http"], 403);
    let server_err = execute(
        Command::View {
            issue: IssueRef::parse("500").unwrap(),
        },
        ctx(&server),
    )
    .await
    .unwrap_err();
    assert_eq!(server_err.exit_code(), 2);
    assert_eq!(server_err.to_json()["http"], 500);
}

#[tokio::test(flavor = "current_thread")]
async fn close_conflict_is_exit_4() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/api/v1/repos/sid/hello-world/issues/9"))
        .respond_with(ResponseTemplate::new(409))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/issues/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(issue_json(9, "x")))
        .mount(&server)
        .await;
    empty_deps(&server, 9).await;
    let err = execute(
        Command::Close {
            issue: IssueRef::parse("9").unwrap(),
        },
        ctx(&server),
    )
    .await
    .unwrap_err();
    assert_eq!(err.exit_code(), 4);
    assert_eq!(err.to_json()["code"], "conflict");
}

#[tokio::test(flavor = "current_thread")]
async fn create_enrichment_failure_keeps_identity() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/sid/hello-world/issues"))
        .respond_with(ResponseTemplate::new(201).set_body_json(issue_json(11, "new")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/issues/11/dependencies"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let err = execute(
        Command::Create {
            title: "new".into(),
            body: "b".into(),
            labels: vec![],
            assignees: vec![],
            blocked_by: vec![],
        },
        ctx(&server),
    )
    .await
    .unwrap_err();
    match err {
        FjiError::Uncertain {
            issue_number,
            issue_url,
            ..
        } => {
            assert_eq!(issue_number, Some(11));
            assert!(issue_url.unwrap().contains("/issues/11"));
        }
        other => panic!("expected uncertain, got {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn label_on_later_page_is_resolved() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/labels"))
        .and(query_param("page", "1"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-total-count", "2")
                .set_body_json(json!([{"id": 1, "name": "keep"}])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/labels"))
        .and(query_param("page", "2"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-total-count", "2")
                .set_body_json(json!([{"id": 2, "name": "later"}])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/labels"))
        .and(query_param("page", "3"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-total-count", "2")
                .set_body_json(json!([])),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/sid/hello-world/issues"))
        .respond_with(ResponseTemplate::new(201).set_body_json(issue_json(12, "labelled")))
        .mount(&server)
        .await;
    empty_deps(&server, 12).await;
    let value = execute(
        Command::Create {
            title: "labelled".into(),
            body: "b".into(),
            labels: vec!["later".into()],
            assignees: vec![],
            blocked_by: vec![],
        },
        ctx(&server),
    )
    .await
    .unwrap();
    assert_eq!(value["number"], 12);
}

#[tokio::test(flavor = "current_thread")]
async fn view_paginates_comments_and_strips_email() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/issues/3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(issue_json(3, "talk")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/issues/3/comments"))
        .and(query_param("page", "1"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-total-count", "3")
                .set_body_json(json!([comment_json(1, "one"), comment_json(2, "two")])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/issues/3/comments"))
        .and(query_param("page", "2"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-total-count", "3")
                .set_body_json(json!([comment_json(3, "three")])),
        )
        .mount(&server)
        .await;
    empty_deps(&server, 3).await;
    let value = execute(
        Command::View {
            issue: IssueRef::parse("3").unwrap(),
        },
        ctx(&server),
    )
    .await
    .unwrap();
    assert_eq!(value["comments"].as_array().unwrap().len(), 3);
    assert!(value.get("milestone").is_none());
    let dump = value.to_string();
    assert!(!dump.contains("hidden@example.com"));
    assert!(!dump.contains("ignore-me"));
}

#[tokio::test(flavor = "current_thread")]
async fn assign_keeps_existing_and_adds() {
    let server = MockServer::start().await;
    let mut issue = issue_json(4, "claim");
    issue["assignees"] = json!([{"login": "alice"}]);
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/issues/4"))
        .respond_with(ResponseTemplate::new(200).set_body_json(issue.clone()))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/api/v1/repos/sid/hello-world/issues/4"))
        .respond_with({
            let mut updated = issue.clone();
            updated["assignees"] = json!([{"login": "alice"}, {"login": "sid"}]);
            ResponseTemplate::new(201).set_body_json(updated)
        })
        .mount(&server)
        .await;
    empty_deps(&server, 4).await;
    let value = execute(
        Command::Assign {
            issue: IssueRef::parse("4").unwrap(),
            users: vec!["sid".into()],
        },
        ctx(&server),
    )
    .await
    .unwrap();
    let assignees = value["assignees"].as_array().unwrap();
    assert!(assignees.iter().any(|a| a == "alice"));
    assert!(assignees.iter().any(|a| a == "sid"));
    assert!(value.get("comments").is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn comment_and_edit_projections() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/sid/hello-world/issues/6/comments"))
        .respond_with(ResponseTemplate::new(201).set_body_json(comment_json(88, "hi")))
        .mount(&server)
        .await;
    let comment = execute(
        Command::Comment {
            issue: IssueRef::parse("6").unwrap(),
            body: "hi".into(),
        },
        ctx(&server),
    )
    .await
    .unwrap();
    assert_eq!(comment["id"], 88);
    assert_eq!(comment["issue"], 6);
    assert_eq!(comment["body"], "hi");
    assert!(!comment.to_string().contains("hidden@example.com"));

    Mock::given(method("PATCH"))
        .and(path("/api/v1/repos/sid/hello-world/issues/6"))
        .respond_with(ResponseTemplate::new(201).set_body_json({
            let mut issue = issue_json(6, "renamed");
            issue["body"] = json!("new body");
            issue
        }))
        .mount(&server)
        .await;
    empty_deps(&server, 6).await;
    let edited = execute(
        Command::Edit {
            issue: IssueRef::parse("6").unwrap(),
            title: Some("renamed".into()),
            body: Some("new body".into()),
        },
        ctx(&server),
    )
    .await
    .unwrap();
    assert_eq!(edited["title"], "renamed");
    assert_eq!(edited["body"], "new body");
    assert!(edited.get("comments").is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn dep_list_has_both_directions() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/issues/20"))
        .respond_with(ResponseTemplate::new(200).set_body_json(issue_json(20, "mid")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/issues/20/dependencies"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([issue_json(18, "blocker")])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/sid/hello-world/issues/20/blocks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([issue_json(21, "waiting")])))
        .mount(&server)
        .await;
    let value = execute(
        Command::DepList {
            issue: IssueRef::parse("20").unwrap(),
        },
        ctx(&server),
    )
    .await
    .unwrap();
    assert_eq!(value["blocked_by"][0]["number"], 18);
    assert_eq!(value["blocks"][0]["number"], 21);
    assert!(value.get("comments").is_none());
}
