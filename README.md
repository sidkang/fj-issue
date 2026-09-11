# fji

Agent-first CLI for Forgejo issues. It talks to `/api/v1` directly. It is not a wrapper around `fj`.

Stdout is always JSON. Stderr is one JSON object on failure. There is no editor, no prompt, and no Unicode decoration.

## Install

```bash
cargo install --path .
```

Binary name: `fji`.

## Auth

```bash
export FJI_TOKEN=...          # preferred
# or
export FORGEJO_TOKEN=...
```

Create a Forgejo application token with issue read/write. `fji` does not log in and does not read `fj`'s key file.

## Addressing

```bash
fji -H git.example -R owner/repo view 12
fji -H git.example view owner/repo#12
```

`-R` is `owner/repo` (GitHub CLI meaning), not a git remote name. A qualified `owner/repo#N` wins; it errors if it disagrees with `-R`.

If you omit `-H`/`-R`, `fji` reads `git remote get-url origin` in `--cwd` (default: current directory). Non-colocated jj repos should pass flags explicitly.

Authenticated requests are HTTPS. Loopback HTTP is allowed for tests. Optional `FJI_TIMEOUT_MS` overrides the 30s per-request deadline (used by tests).

## Commands

```text
fji create --title T --body B [--body-file FILE|-] [--label L]... [--assignee U]... [--blocked-by N]...
fji view {N | owner/repo#N}
fji list [--state open|closed|all] [--label L]... [--assignee U] [--search Q] [--limit N] [--all] [--no-deps]
fji comment {N} --body B [--body-file FILE|-]
fji edit {N} [--title T] [--body B|--body-file FILE|-]
fji label add {N} L [L...]
fji label rm {N} L [L...]
fji assign {N} U [U...]
fji unassign {N} U [U...]
fji close {N}
fji reopen {N}
fji dep add {N} --blocked-by M [--blocked-by M...]
fji dep rm {N} --blocked-by M
fji dep list {N}
```

`list` default is `--state open --limit 30`. `--all` walks every page; that is expensive because each issue also fetches dependencies unless you pass `--no-deps`. `--no-deps` **omits** `blocked_by`/`blocks` (unknown), it does not mean unblocked.

`--search` is Forgejo's `q` parameter, not GitHub issue search. Prefer `--label` / `--state` / `--assignee` for agent queries.

## Partial create

`create --blocked-by` creates the issue first, then adds dependencies. If a dependency fails, the issue is kept. The error includes `issue.number`, `applied`, and `failed`. Do **not** retry `create`; run `fji dep add` for the remainder.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | success, `--help`, `--version` |
| 1 | usage / missing host, repo, token, unknown label |
| 2 | HTTP/network, partial write, uncertain transport |
| 3 | 404 |
| 4 | 412 blocked (open dependencies) or 409 conflict |

`--help` and `--version` print human text on stdout. Everything else is JSON.

## Tests

```bash
cargo test
FJI_LIVE=1 FJI_HOST=git.882816.xyz FJI_REPO=sid/hello-world FJI_TOKEN=... cargo test --test live -- --nocapture
```
