# fji — agent-first Forgejo issue CLI

Status: accepted and implemented (v0.1.0). Offline tests pass. Live smoke on sid/hello-world passed (create, blocked-by, assign, comment, edit, close-412, close).

Review disposition: all P1s accepted; P2s accepted (env-only auth; keep `edit` and name it in the outcome; drop `milestone` from the projection; one execution entry point). §11 forks are closed in §11 below.

## 1. Outcome

A Rust CLI named `fji` that an agent can call non-interactively to manage Forgejo issues, with:

- stable JSON on stdout for every command result
- one JSON object on stderr for every failure (including argv/usage errors)
- no editor, no TTY prompts, no Unicode isolates, no ANSI
- one-shot commands covering the tracker loop: create, view, list, comment, edit, label, assign, close, reopen, dependencies
- addressing like `gh` (`-R owner/repo`, `-H host`), not like installed `fj` 0.6.0

Success is observable against `https://git.882816.xyz/sid/hello-world`:

1. create a map issue
2. create a child with `--blocked-by`
3. `list` the frontier (`blocked_by` empty or all closed)
4. `assign` (claim)
5. `comment`
6. `edit` the map body
7. `close` the child after its blocker is closed; `close` of a still-blocked issue exits 4

Proven only after default `cargo test` plus one explicitly authorized live smoke on that disposable repo.

## 2. Why this exists

Installed `fj` 0.6.0 cannot back that loop: no JSON, no dependency commands, create writes to stderr, numbers wrapped in U+2068/U+2069, weak repo addressing.

Forgejo’s API can. This tool is the missing mechanism. It is not a second `fj`, not a pi skill, not a local markdown tracker.

## 3. Constraints

- Language: Rust (owner-required).
- Repo: `/Users/sid/Devs/fj-issue`, remote `https://github.com/sidkang/fj-issue.git` (empty; `.jj/` present — use `jj`).
- Design cores: `gh issue` (addressing, flags, machine output) and `local-issue` (flat verbs, agent CLI habits). `forgejo-cli` is a limited architecture reference (`forgejo-api`, `owner/repo#n` parser, SIGPIPE), not a UX source.
- Agent-first. No pretty-print in v1.
- Do not wrap the `fj` binary. Call `/api/v1` via `forgejo-api` 0.11.x.
- Do not implement Matt skill policy (label vocabulary, `Part of #N`, wayfinder types). CLI provides primitives only.
- Destructive live tests only on disposable repos (`sid/hello-world`).
- Authenticated requests use HTTPS. HTTP is allowed only for loopback mock servers (`127.0.0.1`, `::1`, `localhost`).

## 4. Removal-test scope (v1)

Admitted:

| Command | Why the outcome fails without it |
|---|---|
| `create` | publish the map and child |
| `view` | read a ticket, its comments, and deps |
| `list` | frontier / triage query |
| `comment` | briefs / progress |
| `edit` | update the map body and correct titles after publish |
| `label add\|rm` | triage state |
| `assign` / `unassign` | claim |
| `close` / `reopen` | resolve |
| `dep add\|rm\|list` | native blocking (no sub-issues) |

Rejected (outcome still holds):

- `status`, `progress` alias, `log`
- pin, lock, milestone, due date, attachments, time tracking, reactions
- `auth login`, fj `keys.json` fallback, credential wizards
- `--pretty`, `--jq`, completions, i18n
- PR / wiki / actions / projects
- `--parent` / body rewriting
- `close`/`reopen --comment` (use `comment` then `close`; avoids a second composite mutation)
- wrapping `fj`
- nested `fji issue …` aliases
- automatic mutation retries or rollback-deletes
- a public repository trait / second production client

## 5. Interface

One execution entry point over already-parsed commands:

```
fji_lib::execute(cmd: Command, ctx: Context) -> Result<Value, FjiError>
```

`main.rs`: clap `try_parse`, map `--help`/`--version` to human stdout / exit 0, map every other failure through the JSON error writer, buffer the success `Value` then write stdout once.

Layout:

```
src/
  main.rs       # argv, help/version, byte I/O, exit codes
  lib.rs        # execute()
  cli.rs        # clap types
  config.rs     # host / token / repo resolution after clap
  client.rs     # real forgejo-api client; pagination, labels, deps, 412
  model.rs      # slim JSON types
  error.rs
  git.rs        # origin URL inference via `git` subprocess
```

`client.rs` owns: HTTPS, timeouts, pagination, label-name→id, dependency fan-out, 412 enrichment. `forgejo-api` types stay private to `client.rs`. Tests never replace `client.rs` with a fake; they stand up a mock HTTP server and use the real client.

No public library compatibility promise. The crate is a binary with tests sharing code.

## 6. Command contract

Binary: `fji`. Package: `fj-issue`.

```
fji [-H HOST] [-R owner/repo] [-C DIR] <command>
```

### 6.1 Global flags

| Flag / env | Grammar |
|---|---|
| `-H, --host` / `FJI_HOST` | hostname or `https://hostname` (no path, no `owner/repo`) |
| `-R, --repo` / `FJI_REPO` | exactly `owner/repo` (gh meaning). Not a git remote name. Not a URL. |
| `-C, --cwd` | directory used only for origin inference |
| `FJI_TOKEN` then `FORGEJO_TOKEN` | required for network commands |

No `--json`, no `--color`. Command results are always JSON. Help/version are the only human stdout.

### 6.2 Issue reference grammar

Every command that takes an issue uses the same parser:

- `123` or `#123` — number in the resolved repo
- `owner/repo#123` — qualified reference

Qualified reference is authoritative for owner/repo.

Conflict rules, applied after parsing, before any HTTP:

1. If the specifier is qualified and `-R` / `FJI_REPO` is set to a different `owner/repo` → usage error, `code: "repo_conflict"`, exit 1.
2. If the specifier is qualified and `-R` / `FJI_REPO` matches or is absent → use the specifier’s repo.
3. If the specifier is unqualified → repo from `-R`, else `FJI_REPO`, else origin, else `code: "no_repo"`.

The same grammar applies to `--blocked-by` values: `M` or `owner/repo#M`. Cross-repo `--blocked-by` in v1 is a usage error (`code: "cross_repo_dep"`). Forgejo can link across repos; we are not taking that complexity in v1.

### 6.3 Host and token (after repo is known)

Host, in order:

1. `-H` / `FJI_HOST`
2. origin URL host when inference is used
3. else `code: "no_host"`

Normalize: strip trailing slash; if no scheme, assume `https`. Reject `http` unless the host is loopback. Resolve the token only after host is final. Do not pick a token by scanning `fj` key files. Missing token → `code: "no_token"`, exit 1, message tells the user to export `FJI_TOKEN`.

Origin inference (only when host or repo still missing): `git -C <cwd> remote get-url origin`. Parse `https://host/owner/repo.git` and `git@host:owner/repo.git`. Non-colocated jj repos without a git origin must pass `-H`/`-R` explicitly.

No default host. Instance-generic.

### 6.4 Commands

```
fji create --title T --body B [--body-file FILE|-] [--label L]... [--assignee U]... [--blocked-by N]...
fji view {ISSUE}
fji list [--state open|closed|all] [--label L]... [--assignee U] [--search Q] [--limit N] [--all] [--no-deps]
fji comment {ISSUE} --body B [--body-file FILE|-]
fji edit {ISSUE} [--title T] [--body B|--body-file FILE|-]
fji label add {ISSUE} L [L...]
fji label rm {ISSUE} L [L...]
fji assign {ISSUE} U [U...]
fji unassign {ISSUE} U [U...]
fji close {ISSUE}
fji reopen {ISSUE}
fji dep add {ISSUE} --blocked-by M [--blocked-by M...]
fji dep rm {ISSUE} --blocked-by M
fji dep list {ISSUE}
```

Validation (exit 1, no HTTP):

- `create` requires `--title` and a body (`--body` or `--body-file`). Never open an editor.
- `--body` and `--body-file` conflict.
- `--body-file -` reads stdin; binary stdin is an error.
- `edit` requires at least one of `--title` or body.
- `--limit` default 30, must be ≥ 1. `--all` and `--limit` together are a usage error.
- Unknown `--state` is a usage error.
- Label names on create/label are strings. Missing labels on the repo are `code: "unknown_label"` (exit 1) after a label-list fetch — this is the one validation that needs HTTP, and it must happen **before** creating an issue.

### 6.5 Success shapes

Buffer the full JSON value, then write stdout once and exit 0. A later enrichment/serialization failure must not leave a partial success body.

**Issue projection** (no `milestone`; no raw user objects):

```json
{
  "number": 7,
  "url": "https://host/owner/repo/issues/7",
  "title": "...",
  "body": "...",
  "state": "open",
  "labels": ["wayfinder:task"],
  "assignees": ["sid"],
  "author": "sid",
  "blocked_by": [{"number": 4, "state": "open", "title": "...", "url": "..."}],
  "blocks": [],
  "created_at": "2026-09-11T15:46:31Z",
  "updated_at": "2026-09-11T15:46:31Z"
}
```

Field presence:

- `blocked_by` / `blocks` present and possibly `[]` means **fetched**. Absence means **unknown** (only when `list --no-deps`).
- `view` always fetches comments, fully paginated, as `"comments": [{"id", "author", "body", "created_at"}]`.
- `list` items never include `comments`.

Who returns the issue projection: `create` (without comments), `view` (with comments), `edit`, `label *`, `assign`, `unassign`, `close`, `reopen`, `dep add`, `dep rm`, `dep list` (deps filled; no comments).

`comment` returns the comment object:

```json
{"id": 12, "issue": 7, "author": "sid", "body": "...", "created_at": "..."}
```

`list` returns:

```json
{
  "items": [ /* issue projections, no comments */ ],
  "truncated": false
}
```

`total_count` is included **only** when the API’s `X-Total-Count` is present and the query is not a search whose count the plan cannot trust. If the header is absent, omit the field. Establish `truncated` without the header by requesting `limit+1` and dropping the extra item.

### 6.6 Errors

On failure: no stdout body (or only bytes already flushed before discovering failure — implementations must buffer to make this “none”). Stderr is one JSON object, then a non-zero exit.

Common fields:

```json
{
  "error": "human-readable, stable enough to log",
  "code": "blocked",
  "http": 412
}
```

`http` omitted when there was no HTTP response. Additional fields only when listed below.

| Exit | `code` examples | When |
|---|---|---|
| 0 | — | success; or `--help`/`--version` |
| 1 | `usage`, `repo_conflict`, `no_host`, `no_repo`, `no_token`, `unknown_label`, `cross_repo_dep` | argv, resolution, unknown label |
| 2 | `http`, `network`, `timeout`, `partial_create`, `partial_dep`, `uncertain` | transport, 5xx, other HTTP, partial writes, unknown outcome |
| 3 | `not_found` | HTTP 404 |
| 4 | `blocked` | HTTP 412 on close (open dependencies) |
| 4 | `conflict` | HTTP 409 |

`--help` / `--version`: clap’s human text on stdout, exit 0, no JSON, no network. All other clap failures go through this JSON writer (exit 1, `code: "usage"`).

412 on close: if a follow-up dep fetch works, include `"blocked_by": [numbers]`. If that fetch fails, still exit 4 with `code: "blocked"` and no `blocked_by` rather than masking the 412 as a generic error.

### 6.7 Non-atomic writes

No command is a server transaction. No automatic retries of mutations. No compensating delete.

**create + `--blocked-by`**

1. Resolve labels (fail 1 before create if any name is missing).
2. Create the issue.
3. Add dependencies in argv order. Stop at the first failure.
4. On dep failure: exit 2, `code: "partial_create"`, error JSON includes `"issue": {number, url}`, `"applied": [M…]`, `"failed": {"number": M, "error": "…", "http": …}`. Callers must **not** retry `create`; they should `dep add` the remainder.

**dep add with multiple `--blocked-by`**

Same stop-on-first-failure. Exit 2, `code: "partial_dep"`, include `"issue"` number/url, `"applied"`, `"failed"`.

**Transport vs rejection**

If the HTTP client cannot tell whether a write landed (timeout after the request was sent, dropped connection with no response): `code: "uncertain"`, exit 2, include whatever identity we already have (e.g. created issue if timeout was on a later dep call). Do not retry.

`close` / `reopen` / `comment` / `edit` / `label` / `assign` are single writes after any read used for label ids.

### 6.8 Pagination and enrichment

Fully paginate: issue list (`--all`), comments on `view`, dependency arrays, repo label lookup. Empty array means fetched-and-empty.

`list` without `--no-deps` issues about `2N` extra requests (dependencies + blocks) plus extra pages. Concurrency cap is 8 in-flight, not a bound on duration. `--all` on a large repo is expensive; document it. One enrichment failure fails the whole `list` (exit 2). Do not emit a partial `items` array.

## 7. Implementation notes

### Crates

- `clap` 4 derive, color off, `try_parse` from `main`
- `forgejo-api` 0.11
- `serde` / `serde_json`
- `tokio` current-thread + join set
- `thiserror`
- `url`
- HTTP mock in tests: `wiremock` or equivalent

No `directories`, `git2`, Fluent, `comrak`, `open`.

### Copy from forgejo-cli

- `forgejo-api` usage
- `owner/repo#n` parser idea
- clap derive
- SIGPIPE → exit, don’t panic

### Do not copy

- Fluent, fancy Unicode, editor, `--web`, `-R` as git remote name, prose stdout, token file I/O

### Client

- User-Agent: `fj-issue/<version>`
- 30s per request; `FJI_TIMEOUT_MS` if it stays a one-line env read
- Never create labels implicitly

## 8. Tests

Default `cargo test` is offline.

Stand up a mock HTTP server **under** the real `forgejo-api` client. Add tests in the same step as each command, not as a later dump.

Required cases:

- id / host / repo / origin-url parsing
- qualified specifier vs `-R` match and conflict
- JSON snapshots of the issue projection (no email keys, no `milestone`)
- clap: missing title, body+body-file, `--all --limit`, `--limit 0` → stderr JSON exit 1
- `--help` human stdout exit 0
- binary-level stdout/stderr/exit (not only `FjiError` conversion)
- `--body-file -` / file path
- multi-page comments, deps, labels
- `list` without `X-Total-Count` (`limit+1`)
- `list --no-deps` omits dep fields
- one enrichment failure fails `list`
- unknown label before create (no POST /issues)
- create then partial dep failure (issue exists in mock; error contains number)
- close 412 with blocker re-fetch, and 412 with re-fetch failure
- `uncertain` mapping for a dropped mutation response

Live: `FJI_LIVE=1` plus explicit `FJI_HOST` / `FJI_REPO` / `FJI_TOKEN` (no implicit production target). Owner-authorized smoke on `sid/hello-world` is required before calling v1 done. Steps: create, blocked child, list frontier, assign, comment, edit, close-blocked→exit 4, close blocker, close child.

## 9. Docs in-repo (v1)

- `README.md`: install, `FJI_TOKEN`, command table, JSON examples, exit codes, partial-create recovery, cost of `list --all`, “not a fj wrapper”
- `AGENTS.md`: JSON stdout, no prompts, `-R`/`-H`, never retry `create` after `partial_create`, `--no-deps` means unknown not unblocked
- This `PLAN.md` stays as the design record

No pi skill in this repo.

## 10. Implementation order (after acceptance)

Tests ride along with each step.

1. Skeleton: clap `try_parse`, JSON error writer, help/version, SIGPIPE
2. config resolution + conflict rules (unit tests, no HTTP)
3. client + mock server + `view`
4. `comment` / `edit` / `label` / `assign`
5. `create` (labels-first) + `--blocked-by` partial_create
6. `close` / `reopen` + 412
7. `dep *` + partial_dep
8. `list` + enrichment + truncation
9. README / AGENTS.md
10. Authorized live smoke on hello-world

One writer. `jj describe` the working copy. Do not push until asked.

## 11. Closed forks (Astra)

| # | Decision |
|---|---|
| 1 | Flat `fji create` |
| 2 | Always JSON for command results; help/version exempt |
| 3 | Env-only tokens in v1 |
| 4 | `view` always includes fully paginated comments; no `--comments` flag |
| 5 | Dep enrichment default on; `--no-deps` omits fields |
| 6 | No rollback on create+dep failure; return identity + applied + failed |
| 7 | Exit 4 for 412 `blocked`; JSON `code` distinguishes `blocked` vs `conflict` |
| 8 | Tokio current-thread |
| 9 | Origin only; explicit flags otherwise |

## 12. Evidence

- Forgejo 16.0.2 `git.882816.xyz`: native deps, 412 on close, no sub-issues.
- `gh` 2.100: `-R`, `--json`, `--blocked-by`; GitHub close does not enforce blockers.
- `local-issue`: flat verbs, non-interactive.
- `forgejo-api` 0.11: typed dependency list/add/remove.
- Installed `fj` 0.6.0 is not an agent JSON tool.

## 13. Do not add

Nested aliases, Matt vocabulary, auth wizard, fj key scrape, generic repo trait, raw API dumps, mutation retries, rollback delete, dashboard, embedded jq, pretty tables, completions, configurable concurrency, `--insecure` (loopback HTTP only).
