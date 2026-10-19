use std::cell::OnceCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::cache::Cache;

const API: &str = "https://api.github.com";
const PER_PAGE: usize = 100;
const MAX_PAGES: u32 = 50;
const ATTEMPTS: u32 = 3;

type Response = ureq::http::Response<ureq::Body>;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct User {
    pub login: String,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub html_url: String,
    pub public_repos: u32,
    pub followers: u32,
    pub following: u32,
    pub created_at: String,
}

impl User {
    pub fn is_org(&self) -> bool {
        self.kind == "Organization"
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Repo {
    pub name: String,
    pub html_url: String,
    pub description: Option<String>,
    pub language: Option<String>,
    pub stargazers_count: u32,
    pub forks_count: u32,
    pub fork: bool,
    pub archived: bool,
    #[serde(default)]
    pub private: bool,
    pub created_at: Option<String>,
    pub pushed_at: Option<String>,
}

pub struct Client {
    agent: ureq::Agent,
    token: Option<String>,
    cache: Cache,
    me: OnceCell<Option<String>>,
}

impl Client {
    pub fn new(token: Option<String>, cache: Cache) -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(20)))
            .build();
        Self {
            agent: config.into(),
            token,
            cache,
            me: OnceCell::new(),
        }
    }

    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    pub fn user(&self, login: &str) -> Result<User> {
        if !valid_login(login) {
            bail!("'{login}' is not a valid GitHub username");
        }
        self.get(&format!("/users/{login}")).map_err(|e| {
            if e.to_string().starts_with("not found") {
                anyhow!("no GitHub user or org called '{login}'")
            } else {
                e
            }
        })
    }

    pub fn me(&self) -> Result<String> {
        if self.token.is_none() {
            bail!("no username given and not logged in, pass a name or run `gh auth login`");
        }
        let user: User = self.get("/user")?;
        Ok(user.login)
    }

    fn is_me(&self, login: &str) -> bool {
        self.me
            .get_or_init(|| self.token.as_ref().and_then(|_| self.me().ok()))
            .as_deref()
            .is_some_and(|me| me.eq_ignore_ascii_case(login))
    }

    // private repos show up for your own account and orgs you're in
    pub fn repos(&self, user: &User, with_private: bool) -> Result<Vec<Repo>> {
        let login = &user.login;
        let base = if user.is_org() {
            let kind = if with_private { "all" } else { "public" };
            format!("/orgs/{login}/repos?type={kind}")
        } else if with_private && self.is_me(login) {
            "/user/repos?affiliation=owner&visibility=all".to_string()
        } else {
            format!("/users/{login}/repos?type=owner")
        };
        paginate(|page| {
            self.get(&format!(
                "{base}&sort=pushed&per_page={PER_PAGE}&page={page}"
            ))
        })
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{API}{path}");
        let body = self.fetch(&url, "", path, || {
            self.with_headers(self.agent.get(&url)).call()
        })?;
        serde_json::from_str(&body)
            .with_context(|| format!("could not parse the response for {path}"))
    }

    pub fn graphql(&self, query: &str, vars: Value) -> Result<Value> {
        if self.token.is_none() {
            bail!("the GraphQL API needs a token");
        }
        let url = format!("{API}/graphql");
        let payload = serde_json::json!({ "query": query, "variables": vars });
        let body = self.fetch(&url, &payload.to_string(), "graphql", || {
            self.with_headers(self.agent.post(&url)).send_json(&payload)
        })?;

        let mut resp: Value =
            serde_json::from_str(&body).context("could not parse the GraphQL response")?;
        let data = resp["data"].take();
        if data.is_null() {
            let msg = resp["errors"][0]["message"]
                .as_str()
                .unwrap_or("unknown error");
            bail!("GraphQL error: {msg}");
        }
        Ok(data)
    }

    fn with_headers<B>(&self, req: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        let req = req
            .header("User-Agent", "gh-stats")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        match &self.token {
            Some(token) => req.header("Authorization", format!("Bearer {token}")),
            None => req,
        }
    }

    fn fetch(
        &self,
        url: &str,
        body: &str,
        what: &str,
        call: impl Fn() -> Result<Response, ureq::Error>,
    ) -> Result<String> {
        let key = self.cache_key(url, body);
        let now = now_secs();
        if let Some(hit) = self.cache.get(&key, now) {
            return Ok(hit);
        }

        let mut attempt = 1;
        let mut resp = loop {
            match call() {
                Ok(r) if !(500..600).contains(&r.status().as_u16()) || attempt == ATTEMPTS => {
                    break r;
                }
                Err(e) if attempt == ATTEMPTS => {
                    return Err(anyhow!(e).context("could not reach api.github.com"));
                }
                _ => {}
            }
            sleep(Duration::from_millis(400 * 2u64.pow(attempt)));
            attempt += 1;
        };

        match resp.status().as_u16() {
            200 => {}
            401 => bail!("GitHub rejected the token, check GITHUB_TOKEN or run `gh auth login`"),
            403 | 429 if header(&resp, "x-ratelimit-remaining").as_deref() == Some("0") => {
                let reset = header(&resp, "x-ratelimit-reset").and_then(|v| v.parse().ok());
                bail!("{}", rate_limit_message(reset, now, self.token.is_some()))
            }
            404 => bail!("not found: {what}"),
            code => bail!("GitHub returned {code} for {what}"),
        }

        let text = resp
            .body_mut()
            .read_to_string()
            .with_context(|| format!("could not read the response for {what}"))?;
        self.cache.put(&key, &text, now);
        Ok(text)
    }

    // different tokens can see different repos, so they get separate entries
    fn cache_key(&self, url: &str, body: &str) -> String {
        let mut h = DefaultHasher::new();
        self.token.hash(&mut h);
        format!("{url}\n{body}\n{:x}", h.finish())
    }
}

pub fn find_token() -> Option<String> {
    for var in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if let Ok(token) = std::env::var(var) {
            let token = token.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    let out = Command::new("gh").args(["auth", "token"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let token = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!token.is_empty()).then_some(token)
}

pub fn valid_login(login: &str) -> bool {
    !login.is_empty()
        && login.len() <= 39
        && !login.starts_with('-')
        && !login.ends_with('-')
        && login
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn paginate<T>(mut fetch: impl FnMut(u32) -> Result<Vec<T>>) -> Result<Vec<T>> {
    let mut all = Vec::new();
    for page in 1..=MAX_PAGES {
        let batch = fetch(page)?;
        let last = batch.len() < PER_PAGE;
        all.extend(batch);
        if last {
            break;
        }
    }
    Ok(all)
}

fn rate_limit_message(reset: Option<u64>, now: u64, has_token: bool) -> String {
    let mut msg = String::from("hit the GitHub rate limit");
    if let Some(reset) = reset.filter(|r| *r > now) {
        msg += &format!(", it resets in {} min", (reset - now).div_ceil(60));
    }
    if !has_token {
        msg += ". Set GITHUB_TOKEN or run `gh auth login` to get 5000 requests an hour";
    }
    msg
}

fn header(resp: &Response, name: &str) -> Option<String> {
    resp.headers().get(name)?.to_str().ok().map(str::to_string)
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_validation() {
        for ok in ["CanReader", "rust-lang", "a", "x1-y2", "jdoe_acme"] {
            assert!(valid_login(ok), "{ok}");
        }
        for bad in [
            "",
            "-abc",
            "abc-",
            "a/b",
            "a b",
            "../users",
            &"a".repeat(40),
        ] {
            assert!(!valid_login(bad), "{bad}");
        }
    }

    #[test]
    fn paginate_stops_on_short_page() {
        let mut calls = Vec::new();
        let all = paginate(|page| {
            calls.push(page);
            Ok(match page {
                1 | 2 => vec![0; PER_PAGE],
                _ => vec![0; 7],
            })
        })
        .unwrap();
        assert_eq!(all.len(), 2 * PER_PAGE + 7);
        assert_eq!(calls, [1, 2, 3]);
    }

    #[test]
    fn paginate_has_a_page_limit() {
        let all = paginate(|_| Ok(vec![0; PER_PAGE])).unwrap();
        assert_eq!(all.len(), MAX_PAGES as usize * PER_PAGE);
    }

    #[test]
    fn paginate_passes_errors_up() {
        let res: Result<Vec<u8>> = paginate(|_| bail!("boom"));
        assert!(res.is_err());
    }

    #[test]
    fn rate_limit_text() {
        let msg = rate_limit_message(Some(1_000 + 90), 1_000, false);
        assert!(msg.contains("resets in 2 min"));
        assert!(msg.contains("gh auth login"));

        let msg = rate_limit_message(Some(10), 1_000, true);
        assert_eq!(msg, "hit the GitHub rate limit");
    }

    #[test]
    fn cache_key_depends_on_token() {
        let a = Client::new(Some("one".into()), Cache::new(false));
        let b = Client::new(Some("two".into()), Cache::new(false));
        assert_ne!(a.cache_key("u", ""), b.cache_key("u", ""));
        assert!(!a.cache_key("u", "").contains("one"));
    }
}
