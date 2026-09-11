use std::path::PathBuf;

use url::Url;

use crate::error::FjiError;
use crate::git;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRef {
    pub repo: Option<RepoName>,
    pub number: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoName {
    pub owner: String,
    pub name: String,
}

impl RepoName {
    pub fn parse(s: &str) -> Result<Self, FjiError> {
        let s = s.trim().trim_start_matches('/').trim_end_matches('/');
        let Some((owner, name)) = s.split_once('/') else {
            return Err(FjiError::usage(
                "usage",
                format!("repo must be owner/repo, got {s:?}"),
            ));
        };
        if owner.is_empty() || name.is_empty() || name.contains('/') {
            return Err(FjiError::usage(
                "usage",
                format!("repo must be owner/repo, got {s:?}"),
            ));
        }
        if s.contains("://") || s.contains('@') {
            return Err(FjiError::usage("usage", "-R accepts owner/repo, not a URL"));
        }
        Ok(Self {
            owner: owner.to_string(),
            name: name.to_string(),
        })
    }

    pub fn as_pair(&self) -> (&str, &str) {
        (&self.owner, &self.name)
    }
}

impl std::fmt::Display for RepoName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

impl IssueRef {
    pub fn parse(s: &str) -> Result<Self, FjiError> {
        let s = s.trim();
        let (repo, number) = match s.rsplit_once('#') {
            Some((repo, number)) if !repo.is_empty() => (Some(RepoName::parse(repo)?), number),
            Some((_, number)) => (None, number),
            None => (None, s),
        };
        let number = number
            .trim()
            .parse::<i64>()
            .map_err(|_| FjiError::usage("usage", format!("invalid issue number in {s:?}")))?;
        if number <= 0 {
            return Err(FjiError::usage(
                "usage",
                format!("issue number must be positive, got {number}"),
            ));
        }
        Ok(Self { repo, number })
    }
}

#[derive(Debug, Clone)]
pub struct Context {
    pub host: Option<String>,
    pub repo: Option<String>,
    pub cwd: PathBuf,
    pub token: Option<String>,
    pub git_origin: Option<String>,
    pub allow_http: bool,
}

impl Default for Context {
    fn default() -> Self {
        Self {
            host: None,
            repo: None,
            cwd: PathBuf::from("."),
            token: None,
            git_origin: None,
            allow_http: false,
        }
    }
}

impl Context {
    pub fn from_env(host: Option<String>, repo: Option<String>, cwd: Option<PathBuf>) -> Self {
        Self {
            host: host.or_else(|| std::env::var("FJI_HOST").ok()),
            repo: repo.or_else(|| std::env::var("FJI_REPO").ok()),
            cwd: cwd
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))),
            token: std::env::var("FJI_TOKEN")
                .ok()
                .or_else(|| std::env::var("FORGEJO_TOKEN").ok()),
            git_origin: None,
            allow_http: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Resolved {
    pub host_url: Url,
    pub repo: RepoName,
    pub token: String,
}

pub fn resolve(
    ctx: &Context,
    issue: Option<&IssueRef>,
    extra_refs: &[IssueRef],
) -> Result<Resolved, FjiError> {
    let flag_repo = match ctx.repo.as_deref() {
        Some(s) => Some(RepoName::parse(s)?),
        None => None,
    };

    let mut spec_repo: Option<&RepoName> = None;
    if let Some(issue) = issue
        && let Some(repo) = issue.repo.as_ref()
    {
        spec_repo = Some(repo);
    }
    for r in extra_refs {
        if let Some(repo) = r.repo.as_ref() {
            if let Some(existing) = spec_repo {
                if existing != repo {
                    return Err(FjiError::usage(
                        "cross_repo_dep",
                        "v1 does not support cross-repo dependencies",
                    ));
                }
            } else {
                spec_repo = Some(repo);
            }
        }
    }

    if let (Some(flag), Some(spec)) = (flag_repo.as_ref(), spec_repo)
        && flag != spec
    {
        return Err(FjiError::usage(
            "repo_conflict",
            format!("-R {flag} disagrees with issue reference {spec}"),
        ));
    }

    let origin = if ctx.host.is_none() || (flag_repo.is_none() && spec_repo.is_none()) {
        match ctx.git_origin.as_deref() {
            Some(s) => Some(parse_origin(s)?),
            None => git::origin_url(&ctx.cwd)
                .as_deref()
                .map(parse_origin)
                .transpose()?,
        }
    } else {
        None
    };

    let repo = if let Some(r) = spec_repo {
        r.clone()
    } else if let Some(r) = flag_repo {
        r
    } else if let Some((_, repo)) = origin.as_ref() {
        repo.clone()
    } else {
        return Err(FjiError::usage(
            "no_repo",
            "pass -R owner/repo or owner/repo#N, or run inside a git checkout with origin",
        ));
    };

    let host_raw = if let Some(h) = ctx.host.as_deref() {
        h.to_string()
    } else if let Some((host, _)) = origin {
        host
    } else {
        return Err(FjiError::usage(
            "no_host",
            "pass -H host or run inside a git checkout with origin",
        ));
    };

    let host_url = normalize_host(&host_raw, ctx.allow_http)?;
    let token = ctx.token.clone().filter(|t| !t.is_empty()).ok_or_else(|| {
        FjiError::usage(
            "no_token",
            "set FJI_TOKEN or FORGEJO_TOKEN to a Forgejo application token",
        )
    })?;

    Ok(Resolved {
        host_url,
        repo,
        token,
    })
}

pub fn normalize_host(raw: &str, allow_http: bool) -> Result<Url, FjiError> {
    let trimmed = raw.trim().trim_end_matches('/');
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let url = Url::parse(&with_scheme)
        .map_err(|_| FjiError::usage("usage", format!("invalid host {raw:?}")))?;
    if url.path() != "/" && !url.path().is_empty() {
        return Err(FjiError::usage("usage", "host must not include a path"));
    }
    match url.scheme() {
        "https" => Ok(url),
        "http" if allow_http || is_loopback(&url) => Ok(url),
        "http" => Err(FjiError::usage(
            "usage",
            "authenticated requests require HTTPS (loopback HTTP is allowed for tests)",
        )),
        other => Err(FjiError::usage(
            "usage",
            format!("unsupported host scheme {other}"),
        )),
    }
}

fn is_loopback(url: &Url) -> bool {
    matches!(url.host_str(), Some("127.0.0.1" | "::1" | "localhost"))
}

pub fn parse_origin(url: &str) -> Result<(String, RepoName), FjiError> {
    let url = url.trim();
    if let Some(rest) = url.strip_prefix("git@") {
        let Some((host, path)) = rest.split_once(':') else {
            return Err(FjiError::usage(
                "usage",
                format!("could not parse git origin {url:?}"),
            ));
        };
        return Ok((host.to_string(), repo_from_path(path)?));
    }
    let parsed = Url::parse(url)
        .map_err(|_| FjiError::usage("usage", format!("could not parse git origin {url:?}")))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| FjiError::usage("usage", format!("origin has no host: {url:?}")))?
        .to_string();
    let path = parsed.path();
    Ok((host, repo_from_path(path)?))
}

fn repo_from_path(path: &str) -> Result<RepoName, FjiError> {
    let path = path.trim_start_matches('/').trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    RepoName::parse(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_issue_number_and_qualified() {
        let a = IssueRef::parse("12").unwrap();
        assert_eq!(a.number, 12);
        assert!(a.repo.is_none());
        let b = IssueRef::parse("#7").unwrap();
        assert_eq!(b.number, 7);
        let c = IssueRef::parse("sid/hello-world#3").unwrap();
        assert_eq!(c.number, 3);
        assert_eq!(c.repo.unwrap().to_string(), "sid/hello-world");
    }

    #[test]
    fn repo_rejects_url() {
        let err = RepoName::parse("https://git.example/sid/hello-world").unwrap_err();
        assert!(matches!(err, FjiError::Usage { code: "usage", .. }));
    }

    #[test]
    fn qualified_beats_matching_flag() {
        let ctx = Context {
            host: Some("git.example".into()),
            repo: Some("sid/hello-world".into()),
            token: Some("tok".into()),
            ..Context::default()
        };
        let issue = IssueRef::parse("sid/hello-world#4").unwrap();
        let resolved = resolve(&ctx, Some(&issue), &[]).unwrap();
        assert_eq!(resolved.repo.to_string(), "sid/hello-world");
        assert_eq!(resolved.host_url.as_str(), "https://git.example/");
    }

    #[test]
    fn qualified_conflicts_with_flag() {
        let ctx = Context {
            host: Some("git.example".into()),
            repo: Some("other/repo".into()),
            token: Some("tok".into()),
            ..Context::default()
        };
        let issue = IssueRef::parse("sid/hello-world#4").unwrap();
        let err = resolve(&ctx, Some(&issue), &[]).unwrap_err();
        assert!(matches!(
            err,
            FjiError::Usage {
                code: "repo_conflict",
                ..
            }
        ));
    }

    #[test]
    fn origin_fills_host_and_repo() {
        let ctx = Context {
            token: Some("tok".into()),
            git_origin: Some("https://git.882816.xyz/sid/hello-world.git".into()),
            ..Context::default()
        };
        let resolved = resolve(&ctx, None, &[]).unwrap();
        assert_eq!(resolved.repo.to_string(), "sid/hello-world");
        assert_eq!(resolved.host_url.as_str(), "https://git.882816.xyz/");
    }

    #[test]
    fn ssh_origin() {
        let (host, repo) = parse_origin("git@git.882816.xyz:sid/hello-world.git").unwrap();
        assert_eq!(host, "git.882816.xyz");
        assert_eq!(repo.to_string(), "sid/hello-world");
    }

    #[test]
    fn http_non_loopback_rejected() {
        let err = normalize_host("http://git.example", false).unwrap_err();
        assert!(matches!(err, FjiError::Usage { .. }));
    }

    #[test]
    fn http_loopback_ok() {
        let url = normalize_host("http://127.0.0.1:1234", false).unwrap();
        assert_eq!(url.scheme(), "http");
    }

    #[test]
    fn missing_token() {
        let ctx = Context {
            host: Some("git.example".into()),
            repo: Some("sid/hello".into()),
            ..Context::default()
        };
        let err = resolve(&ctx, None, &[]).unwrap_err();
        assert!(matches!(
            err,
            FjiError::Usage {
                code: "no_token",
                ..
            }
        ));
    }

    #[test]
    fn cross_repo_dep_rejected() {
        let ctx = Context {
            host: Some("git.example".into()),
            token: Some("tok".into()),
            ..Context::default()
        };
        let issue = IssueRef::parse("sid/hello#1").unwrap();
        let dep = IssueRef::parse("other/repo#2").unwrap();
        let err = resolve(&ctx, Some(&issue), &[dep]).unwrap_err();
        assert!(matches!(
            err,
            FjiError::Usage {
                code: "cross_repo_dep",
                ..
            }
        ));
    }
}
