# Agent contract for `fji`

- Always pass `-H` and `-R` unless you are certain origin is the Forgejo repo you want.
- Parse stdout as JSON. Do not scrape human text.
- On failure, parse stderr as JSON. Branch on `code` and process exit status.
- Never retry `create` after `code: "partial_create"`. Use `dep add` with the returned issue number.
- `blocked_by: []` means fetched and empty. Missing `blocked_by` (`list --no-deps`) means unknown, not unblocked.
- Do not pass `--body` without a value. Use `--body-file -` for stdin.
- Do not open an editor. `fji` will not.
- Do not assume GitHub sub-issues. Parent/child is policy in the issue body; blocking is `dep`.
- `list --all` plus dependency enrichment is O(n) HTTP. Prefer labels and `--limit` for frontier queries.
