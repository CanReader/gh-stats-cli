use std::collections::HashMap;

use serde::Serialize;

use crate::github::Repo;
use crate::graphql::{Day, RepoLangs};

const ACTIVE_DAYS: i64 = 30;

#[derive(Debug, PartialEq, Serialize)]
pub struct Language {
    pub name: String,
    pub repos: usize,
    pub bytes: Option<u64>,
    pub percent: f64,
    pub color: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Stats {
    // same number GitHub shows on the profile
    pub total: usize,
    pub own: usize,
    pub fork_repos: usize,
    pub repos: usize,
    pub public: usize,
    pub private: usize,
    pub stars: u32,
    pub forks: u32,
    pub archived: usize,
    pub active_last_30_days: usize,
    pub languages: Vec<Language>,
    pub top: Vec<Repo>,
    pub last_push: Option<Repo>,
    pub newest: Option<Repo>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Streaks {
    pub current: u32,
    pub longest: u32,
    pub best_day: Option<(String, u32)>,
}

pub fn collect(
    all: &[Repo],
    include_forks: bool,
    top_n: usize,
    now_secs: u64,
    sizes: Option<&[RepoLangs]>,
) -> Stats {
    let repos: Vec<&Repo> = all.iter().filter(|r| include_forks || !r.fork).collect();
    let own: Vec<&Repo> = all.iter().filter(|r| !r.fork).collect();
    let today = now_secs as i64 / 86_400;

    let mut top: Vec<Repo> = repos
        .iter()
        .filter(|r| r.stargazers_count > 0)
        .map(|&r| r.clone())
        .collect();
    top.sort_by(|a, b| {
        b.stargazers_count
            .cmp(&a.stargazers_count)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    top.truncate(top_n);

    // ISO 8601 dates, so string order is time order
    let latest_by = |key: fn(&Repo) -> Option<&str>| {
        repos
            .iter()
            .filter(|r| key(r).is_some())
            .max_by(|a, b| key(a).cmp(&key(b)))
            .map(|&r| r.clone())
    };

    let active = repos
        .iter()
        .filter_map(|r| r.pushed_at.as_deref().and_then(days))
        .filter(|d| today - d <= ACTIVE_DAYS)
        .count();

    let languages = match sizes {
        Some(sizes) => languages_by_size(sizes),
        None => languages_by_repo(&repos),
    };

    Stats {
        total: all.len(),
        own: own.len(),
        fork_repos: all.len() - own.len(),
        repos: repos.len(),
        public: own.iter().filter(|r| !r.private).count(),
        private: own.iter().filter(|r| r.private).count(),
        stars: repos.iter().map(|r| r.stargazers_count).sum(),
        forks: repos.iter().map(|r| r.forks_count).sum(),
        archived: repos.iter().filter(|r| r.archived).count(),
        active_last_30_days: active,
        languages,
        top,
        last_push: latest_by(|r| r.pushed_at.as_deref()),
        newest: latest_by(|r| r.created_at.as_deref()),
    }
}

fn languages_by_repo(repos: &[&Repo]) -> Vec<Language> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for lang in repos.iter().filter_map(|r| r.language.as_deref()) {
        *counts.entry(lang).or_default() += 1;
    }
    let total: usize = counts.values().sum();
    let mut langs: Vec<Language> = counts
        .into_iter()
        .map(|(name, n)| Language {
            name: name.to_string(),
            repos: n,
            bytes: None,
            percent: share(n as f64, total as f64),
            color: None,
        })
        .collect();
    langs.sort_by(|a, b| b.repos.cmp(&a.repos).then_with(|| a.name.cmp(&b.name)));
    langs
}

fn languages_by_size(repos: &[RepoLangs]) -> Vec<Language> {
    let mut by_name: HashMap<&str, (u64, usize, Option<&str>)> = HashMap::new();
    for l in repos.iter().flat_map(|r| &r.langs) {
        let entry = by_name.entry(&l.name).or_insert((0, 0, None));
        entry.0 += l.bytes;
        entry.1 += 1;
        entry.2 = entry.2.or(l.color.as_deref());
    }
    let total: u64 = by_name.values().map(|v| v.0).sum();
    let mut langs: Vec<Language> = by_name
        .into_iter()
        .map(|(name, (bytes, n, color))| Language {
            name: name.to_string(),
            repos: n,
            bytes: Some(bytes),
            percent: share(bytes as f64, total as f64),
            color: color.map(str::to_string),
        })
        .collect();
    langs.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
    langs
}

fn share(part: f64, total: f64) -> f64 {
    if total == 0.0 {
        return 0.0;
    }
    (part * 1000.0 / total).round() / 10.0
}

pub fn streaks(days: &[Day]) -> Streaks {
    let mut longest = 0;
    let mut run = 0;
    for d in days {
        run = if d.count > 0 { run + 1 } else { 0 };
        longest = longest.max(run);
    }

    // nothing yet today doesn't break the streak, same as on GitHub
    let rest = match days.split_last() {
        Some((last, before)) if last.count == 0 => before,
        _ => days,
    };
    let current = rest.iter().rev().take_while(|d| d.count > 0).count() as u32;

    let best_day = days
        .iter()
        .filter(|d| d.count > 0)
        .max_by(|a, b| a.count.cmp(&b.count).then_with(|| b.date.cmp(&a.date)))
        .map(|d| (d.date.clone(), d.count));

    Streaks {
        current,
        longest,
        best_day,
    }
}

pub fn days(iso: &str) -> Option<i64> {
    let y: i64 = iso.get(0..4)?.parse().ok()?;
    let m: i64 = iso.get(5..7)?.parse().ok()?;
    let d: i64 = iso.get(8..10)?.parse().ok()?;
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::graphql::LangSize;

    // 2026-10-05T00:00:00Z
    pub const NOW: u64 = 1_791_158_400;

    pub fn repo(name: &str, lang: Option<&str>, stars: u32, fork: bool) -> Repo {
        Repo {
            name: name.to_string(),
            html_url: format!("https://github.com/someone/{name}"),
            description: None,
            language: lang.map(String::from),
            stargazers_count: stars,
            forks_count: 1,
            fork,
            archived: false,
            private: false,
            created_at: None,
            pushed_at: None,
        }
    }

    pub fn sample() -> Vec<Repo> {
        let mut engine = repo("engine", Some("C++"), 4, false);
        engine.pushed_at = Some("2026-08-30T10:00:00Z".into());
        engine.created_at = Some("2024-01-10T10:00:00Z".into());
        let mut cli = repo("cli", Some("Rust"), 7, false);
        cli.pushed_at = Some("2026-10-02T08:00:00Z".into());
        cli.created_at = Some("2025-06-01T08:00:00Z".into());
        cli.description = Some("Small command line tool".into());
        let mut notes = repo("notes", None, 0, false);
        notes.archived = true;
        let mut secret = repo("secret", Some("Rust"), 0, false);
        secret.private = true;
        secret.pushed_at = Some("2026-09-20T08:00:00Z".into());
        secret.created_at = Some("2026-09-01T08:00:00Z".into());
        vec![
            engine,
            repo("tool", Some("Rust"), 2, false),
            notes,
            cli,
            secret,
            repo("someone-elses", Some("Go"), 50, true),
        ]
    }

    pub fn day(date: &str, count: u32) -> Day {
        Day {
            date: date.to_string(),
            count,
            level: count.min(4) as u8,
        }
    }

    fn size(name: &str, bytes: u64) -> LangSize {
        LangSize {
            name: name.to_string(),
            bytes,
            color: Some("#123456".into()),
        }
    }

    #[test]
    fn days_since_epoch() {
        assert_eq!(days("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(days("2000-03-01"), Some(11_017));
        assert_eq!(days("2026-10-05T00:00:00Z"), Some(NOW as i64 / 86_400));
        assert_eq!(days("garbage"), None);
    }

    #[test]
    fn skips_forks_by_default() {
        let s = collect(&sample(), false, 10, NOW, None);
        assert_eq!((s.total, s.own, s.fork_repos), (6, 5, 1));
        assert_eq!(s.repos, 5);
        assert_eq!(s.stars, 13);
        assert_eq!(s.forks, 5);
        assert_eq!(s.archived, 1);
        assert!(s.languages.iter().all(|l| l.name != "Go"));
    }

    #[test]
    fn includes_forks_when_asked() {
        let s = collect(&sample(), true, 10, NOW, None);
        assert_eq!(s.repos, 6);
        assert_eq!((s.total, s.own, s.fork_repos), (6, 5, 1));
        assert_eq!(s.stars, 63);
        assert_eq!(s.top[0].name, "someone-elses");
    }

    #[test]
    fn public_and_private() {
        let s = collect(&sample(), false, 10, NOW, None);
        assert_eq!((s.public, s.private), (4, 1));
    }

    #[test]
    fn language_shares_by_repo() {
        let s = collect(&sample(), false, 10, NOW, None);
        let langs: Vec<(&str, usize, f64)> = s
            .languages
            .iter()
            .map(|l| (l.name.as_str(), l.repos, l.percent))
            .collect();
        assert_eq!(langs, [("Rust", 3, 75.0), ("C++", 1, 25.0)]);
    }

    #[test]
    fn language_shares_by_size() {
        let sizes = vec![
            RepoLangs {
                created_at: "2025-01-01".into(),
                langs: vec![size("C++", 600), size("Cuda", 100)],
            },
            RepoLangs {
                created_at: "2025-02-01".into(),
                langs: vec![size("Rust", 300)],
            },
        ];
        let s = collect(&sample(), false, 10, NOW, Some(&sizes));
        let langs: Vec<(&str, u64, f64)> = s
            .languages
            .iter()
            .map(|l| (l.name.as_str(), l.bytes.unwrap(), l.percent))
            .collect();
        assert_eq!(
            langs,
            [("C++", 600, 60.0), ("Rust", 300, 30.0), ("Cuda", 100, 10.0)]
        );
        assert_eq!(s.languages[0].color.as_deref(), Some("#123456"));
    }

    #[test]
    fn top_repos_skip_zero_stars() {
        let s = collect(&sample(), false, 10, NOW, None);
        let names: Vec<&str> = s.top.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["cli", "engine", "tool"]);

        let s = collect(&sample(), false, 1, NOW, None);
        assert_eq!(s.top.len(), 1);
    }

    #[test]
    fn activity() {
        let s = collect(&sample(), false, 5, NOW, None);
        assert_eq!(s.last_push.unwrap().name, "cli");
        assert_eq!(s.newest.unwrap().name, "secret");
        // cli and secret are within 30 days, engine is not
        assert_eq!(s.active_last_30_days, 2);
    }

    #[test]
    fn empty_input() {
        let s = collect(&[], false, 5, NOW, None);
        assert_eq!(s.repos, 0);
        assert_eq!(s.stars, 0);
        assert!(s.languages.is_empty());
        assert!(s.top.is_empty());
        assert!(s.last_push.is_none());
        assert!(s.newest.is_none());
    }

    #[test]
    fn streak_counting() {
        let days = [
            day("2026-10-01", 3),
            day("2026-10-02", 1),
            day("2026-10-03", 9),
            day("2026-10-04", 0),
            day("2026-10-05", 2),
            day("2026-10-06", 4),
        ];
        let s = streaks(&days);
        assert_eq!((s.current, s.longest), (2, 3));
        assert_eq!(s.best_day, Some(("2026-10-03".into(), 9)));
    }

    #[test]
    fn empty_today_keeps_streak() {
        let days = [
            day("2026-10-04", 1),
            day("2026-10-05", 1),
            day("2026-10-06", 0),
        ];
        assert_eq!(streaks(&days).current, 2);

        let days = [
            day("2026-10-04", 1),
            day("2026-10-05", 0),
            day("2026-10-06", 0),
        ];
        assert_eq!(streaks(&days).current, 0);
    }

    #[test]
    fn no_contributions() {
        let s = streaks(&[day("2026-10-04", 0)]);
        assert_eq!(
            s,
            Streaks {
                current: 0,
                longest: 0,
                best_day: None
            }
        );
        assert_eq!(streaks(&[]).current, 0);
    }
}
