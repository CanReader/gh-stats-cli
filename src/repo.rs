use std::fmt::{self, Write};
use std::thread;
use std::time::Duration;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::colors::Palette;
use crate::github::{Client, valid_login};
use crate::render::{
    bar, date, md_cell, num, plural, size, strip_shortcodes, truncate, write_languages,
};
use crate::stats::Language;

const SPARK: [char; 7] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇'];
const CONTRIBUTORS: usize = 8;
const RELEASES: usize = 3;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Info {
    pub full_name: String,
    pub html_url: String,
    pub description: Option<String>,
    pub language: Option<String>,
    pub stargazers_count: u32,
    pub forks_count: u32,
    #[serde(default)]
    pub subscribers_count: u32,
    pub open_issues_count: u32,
    pub license: Option<License>,
    #[serde(default)]
    pub topics: Vec<String>,
    pub created_at: Option<String>,
    pub pushed_at: Option<String>,
    pub archived: bool,
    pub fork: bool,
    #[serde(default)]
    pub private: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct License {
    pub spdx_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Contributor {
    pub login: String,
    pub contributions: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Release {
    pub tag_name: String,
    pub published_at: Option<String>,
    #[serde(default)]
    pub prerelease: bool,
}

#[derive(Deserialize)]
struct Week {
    total: u32,
}

#[derive(Debug, Serialize)]
pub struct RepoReport {
    pub repo: Info,
    pub languages: Vec<Language>,
    pub contributors: Option<Vec<Contributor>>,
    pub releases: Vec<Release>,
    /// commits per week for the last 52 weeks, None while GitHub is still
    /// counting them
    pub weekly_commits: Option<Vec<u32>>,
}

/// "owner/name" when it looks like a repo, None otherwise.
pub fn parse(s: &str) -> Option<(&str, &str)> {
    let (owner, name) = s.split_once('/')?;
    let name_ok = !name.is_empty()
        && name.len() <= 100
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    (valid_login(owner) && name_ok).then_some((owner, name))
}

pub fn load(client: &Client, owner: &str, name: &str) -> Result<RepoReport> {
    let base = format!("/repos/{owner}/{name}");
    let repo: Info = client.get(&base).map_err(|e| {
        if e.to_string().starts_with("not found") {
            anyhow::anyhow!("no repo called '{owner}/{name}', or it's private")
        } else {
            e
        }
    })?;

    let (languages, contributors, releases, weekly) = thread::scope(|s| {
        let langs = s.spawn(|| client.get::<Map<String, Value>>(&format!("{base}/languages")));
        let people = s.spawn(|| {
            client.get::<Vec<Contributor>>(&format!("{base}/contributors?per_page={CONTRIBUTORS}"))
        });
        let releases =
            s.spawn(|| client.get::<Vec<Release>>(&format!("{base}/releases?per_page={RELEASES}")));
        let weekly = s.spawn(|| weekly_commits(client, &base));
        (
            langs.join().expect("thread panicked"),
            people.join().expect("thread panicked"),
            releases.join().expect("thread panicked"),
            weekly.join().expect("thread panicked"),
        )
    });

    // big repos like the linux kernel refuse to list contributors, the rest
    // of the report is still useful without them
    let contributors = contributors
        .inspect_err(|e| eprintln!("gh-stats: no contributors for {owner}/{name}: {e:#}"))
        .ok();
    let weekly = weekly
        .inspect_err(|e| eprintln!("gh-stats: no commit activity for {owner}/{name}: {e:#}"))
        .ok()
        .flatten();

    Ok(RepoReport {
        repo,
        languages: languages_from(&languages?),
        contributors,
        releases: releases?,
        weekly_commits: weekly,
    })
}

// GitHub computes these in the background and answers 202 until it's done
fn weekly_commits(client: &Client, base: &str) -> Result<Option<Vec<u32>>> {
    let path = format!("{base}/stats/commit_activity");
    for attempt in 0..3 {
        if let Some(weeks) = client.get_ready::<Vec<Week>>(&path)? {
            return Ok(Some(weeks.iter().map(|w| w.total).collect()));
        }
        if attempt < 2 {
            thread::sleep(Duration::from_secs(2));
        }
    }
    Ok(None)
}

fn languages_from(map: &Map<String, Value>) -> Vec<Language> {
    let total: u64 = map.values().filter_map(Value::as_u64).sum();
    let mut langs: Vec<Language> = map
        .iter()
        .filter_map(|(name, bytes)| {
            let bytes = bytes.as_u64()?;
            Some(Language {
                name: name.clone(),
                repos: 1,
                bytes: Some(bytes),
                percent: if total == 0 {
                    0.0
                } else {
                    (bytes as f64 * 1000.0 / total as f64).round() / 10.0
                },
                color: None,
            })
        })
        .collect();
    langs.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
    langs
}

pub fn ensure_shareable(r: &RepoReport) -> Result<()> {
    if r.repo.private {
        bail!(
            "{} is private, --md and --svg are meant to be shared so they skip it",
            r.repo.full_name
        );
    }
    Ok(())
}

pub fn text(r: &RepoReport, p: Palette, width: usize) -> String {
    let mut out = String::new();
    write_text(&mut out, r, p, width).expect("writing to a String can't fail");
    out
}

pub fn markdown(r: &RepoReport) -> String {
    let mut out = String::new();
    write_markdown(&mut out, r).expect("writing to a String can't fail");
    out
}

fn write_text(out: &mut String, r: &RepoReport, p: Palette, width: usize) -> fmt::Result {
    let repo = &r.repo;
    write!(out, "  {}", p.bold(&p.fg(p.theme.accent, &repo.full_name)))?;
    if repo.private {
        write!(out, "  {}", p.dim("private"))?;
    }
    writeln!(out)?;
    let desc = strip_shortcodes(repo.description.as_deref().unwrap_or(""));
    if !desc.is_empty() {
        writeln!(out, "  {}", truncate(&desc, width.saturating_sub(4)))?;
    }

    let mut facts = Vec::new();
    if let Some(lang) = &repo.language {
        facts.push(lang.clone());
    }
    if let Some(id) = repo.license.as_ref().and_then(|l| l.spdx_id.as_deref())
        && id != "NOASSERTION"
    {
        facts.push(id.to_string());
    }
    facts.push(format!("created {}", date(&repo.created_at)));
    facts.push(format!("last push {}", date(&repo.pushed_at)));
    if repo.archived {
        facts.push("archived".into());
    }
    if repo.fork {
        facts.push("fork".into());
    }
    writeln!(out, "  {}", p.dim(&facts.join(", ")))?;
    if !repo.topics.is_empty() {
        let topics = format!("topics: {}", repo.topics.join(", "));
        writeln!(
            out,
            "  {}",
            p.dim(&truncate(&topics, width.saturating_sub(4)))
        )?;
    }

    writeln!(
        out,
        "\n  {} stars   {} forks   {} watching   {} open issues and PRs",
        p.bold(&p.fg(p.theme.star, &num(repo.stargazers_count))),
        p.bold(&num(repo.forks_count)),
        p.bold(&num(repo.subscribers_count)),
        p.bold(&num(repo.open_issues_count)),
    )?;

    writeln!(out, "\n  {}", p.bold("Commits"))?;
    match &r.weekly_commits {
        Some(weeks) => {
            let total: u32 = weeks.iter().sum();
            writeln!(
                out,
                "  {}",
                p.dim(&format!(
                    "{} in the last year, one bar per week",
                    num(total)
                ))
            )?;
            writeln!(out, "  {}", sparkline(weeks, p))?;
        }
        None => writeln!(
            out,
            "  {}",
            p.dim("GitHub is still counting them, try again in a minute")
        )?,
    }

    if !r.languages.is_empty() {
        write_languages(out, &r.languages, p)?;
    }

    if let Some(people) = r.contributors.as_ref().filter(|c| !c.is_empty()) {
        writeln!(out, "\n  {}", p.bold("Top contributors"))?;
        let name_w = people
            .iter()
            .map(|c| c.login.chars().count())
            .max()
            .unwrap_or(0);
        let max = people[0].contributions.max(1) as f64;
        for c in people {
            let b = format!("{:<24}", bar(c.contributions as f64 / max, 24));
            writeln!(
                out,
                "  {:<name_w$}  {}  {}",
                c.login,
                p.fg(p.theme.accent, &b),
                p.dim(&plural(c.contributions as usize, "commit")),
            )?;
        }
    }

    if !r.releases.is_empty() {
        writeln!(out, "\n  {}", p.bold("Latest releases"))?;
        let tag_w = r
            .releases
            .iter()
            .map(|r| r.tag_name.chars().count())
            .max()
            .unwrap_or(0);
        for rel in &r.releases {
            let mut when = date(&rel.published_at).to_string();
            if rel.prerelease {
                when += ", pre-release";
            }
            writeln!(out, "  {:<tag_w$}  {}", rel.tag_name, p.dim(&when))?;
        }
    }
    Ok(())
}

fn sparkline(weeks: &[u32], p: Palette) -> String {
    let max = weeks.iter().copied().max().unwrap_or(0);
    weeks
        .iter()
        .map(|&w| {
            if w == 0 || max == 0 {
                return p.fg(p.theme.cells[0], &SPARK[0].to_string());
            }
            let level = (w as usize * SPARK.len()).div_ceil(max as usize);
            let level = level.clamp(1, SPARK.len()) - 1;
            p.fg(p.theme.green, &SPARK[level].to_string())
        })
        .collect()
}

fn write_markdown(out: &mut String, r: &RepoReport) -> fmt::Result {
    let repo = &r.repo;
    writeln!(out, "### [{}]({})\n", repo.full_name, repo.html_url)?;
    if let Some(desc) = repo.description.as_deref().filter(|d| !d.is_empty()) {
        writeln!(out, "{}\n", md_cell(desc))?;
    }
    writeln!(
        out,
        "**{}** stars · **{}** forks · **{}** watching · **{}** open issues and PRs\n",
        num(repo.stargazers_count),
        num(repo.forks_count),
        num(repo.subscribers_count),
        num(repo.open_issues_count)
    )?;
    if let Some(weeks) = &r.weekly_commits {
        let total: u32 = weeks.iter().sum();
        writeln!(out, "**{}** commits in the last year\n", num(total))?;
    }

    if !r.languages.is_empty() {
        out.push_str("| Language | Share | Code |\n| --- | ---: | ---: |\n");
        for l in r.languages.iter().take(8) {
            let code = l.bytes.map(size).unwrap_or_default();
            writeln!(out, "| {} | {:.1}% | {code} |", l.name, l.percent)?;
        }
        out.push('\n');
    }

    if let Some(people) = r.contributors.as_ref().filter(|c| !c.is_empty()) {
        out.push_str("| Contributor | Commits |\n| --- | ---: |\n");
        for c in people {
            writeln!(
                out,
                "| [{0}](https://github.com/{0}) | {1} |",
                c.login,
                num(c.contributions)
            )?;
        }
        out.push('\n');
    }

    out.push_str("<sub>made with [gh-stats](https://github.com/CanReader/gh-stats)</sub>\n");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> RepoReport {
        let repo: Info = serde_json::from_value(json!({
            "full_name": "someone/tool",
            "html_url": "https://github.com/someone/tool",
            "description": ":zap: Fast | small tool",
            "language": "Rust",
            "stargazers_count": 1234,
            "forks_count": 56,
            "subscribers_count": 7,
            "open_issues_count": 8,
            "license": { "spdx_id": "MIT", "name": "MIT License" },
            "topics": ["cli", "github"],
            "created_at": "2024-02-03T00:00:00Z",
            "pushed_at": "2026-10-01T00:00:00Z",
            "archived": false,
            "fork": false
        }))
        .unwrap();
        let langs = json!({ "Rust": 9000, "Shell": 1000 });
        RepoReport {
            repo,
            languages: languages_from(langs.as_object().unwrap()),
            contributors: Some(vec![
                Contributor {
                    login: "someone".into(),
                    contributions: 120,
                },
                Contributor {
                    login: "helper".into(),
                    contributions: 1,
                },
            ]),
            releases: vec![Release {
                tag_name: "v1.2.0".into(),
                published_at: Some("2026-09-30T10:00:00Z".into()),
                prerelease: false,
            }],
            weekly_commits: Some(vec![0, 1, 4, 8]),
        }
    }

    #[test]
    fn repo_names() {
        assert_eq!(parse("rust-lang/rust"), Some(("rust-lang", "rust")));
        assert_eq!(parse("a/b.c_d-e"), Some(("a", "b.c_d-e")));
        for bad in [
            "rust-lang",
            "/rust",
            "rust-lang/",
            "a/b/c",
            "a/..",
            "-a/b",
            "a/b c",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn language_shares() {
        let r = sample();
        let langs: Vec<(&str, f64)> = r
            .languages
            .iter()
            .map(|l| (l.name.as_str(), l.percent))
            .collect();
        assert_eq!(langs, [("Rust", 90.0), ("Shell", 10.0)]);
    }

    #[test]
    fn sparkline_levels() {
        let line = sparkline(&[0, 1, 4, 8], Palette::plain());
        assert_eq!(line, "▁▁▄▇");
    }

    #[test]
    fn plain_report() {
        let out = text(&sample(), Palette::plain(), 100);
        assert!(out.contains("someone/tool"));
        assert!(out.contains("Fast | small tool"));
        assert!(out.contains("Rust, MIT, created 2024-02-03, last push 2026-10-01"));
        assert!(out.contains("topics: cli, github"));
        assert!(out.contains("1,234 stars   56 forks   7 watching   8 open issues and PRs"));
        assert!(out.contains("13 in the last year"));
        assert!(out.contains("Languages  by code size"));
        assert!(out.contains("120 commits"));
        assert!(out.contains("v1.2.0  2026-09-30"));
    }

    #[test]
    fn missing_parts_dont_break_it() {
        let mut r = sample();
        r.contributors = None;
        r.weekly_commits = None;
        r.releases.clear();
        let out = text(&r, Palette::plain(), 100);
        assert!(out.contains("still counting"));
        assert!(!out.contains("Top contributors"));
        assert!(!out.contains("Latest releases"));
    }

    #[test]
    fn markdown_report() {
        let md = markdown(&sample());
        assert!(md.starts_with("### [someone/tool](https://github.com/someone/tool)"));
        assert!(md.contains(":zap: Fast \\| small tool"));
        assert!(md.contains("**1,234** stars"));
        assert!(md.contains("| Rust | 90.0% |"));
        assert!(md.contains("| [someone](https://github.com/someone) | 120 |"));
    }

    #[test]
    fn private_repos_are_not_shared() {
        let mut r = sample();
        assert!(ensure_shareable(&r).is_ok());
        r.repo.private = true;
        assert!(ensure_shareable(&r).is_err());
    }
}
