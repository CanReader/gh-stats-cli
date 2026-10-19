use std::fmt::{self, Write};

use crate::Report;
use crate::colors::{self, Palette};
use crate::heatmap;
use crate::stats::Language;

const ACCENT: (u8, u8, u8) = (0x58, 0xA6, 0xFF);
const STAR: (u8, u8, u8) = (0xE3, 0xB3, 0x41);
const GREEN: (u8, u8, u8) = (0x3F, 0xB9, 0x50);
const MAX_LANGUAGES: usize = 8;
const BAR_CELLS: usize = 24;

pub fn text(r: &Report, p: Palette, width: usize) -> String {
    let mut out = String::new();
    write_text(&mut out, r, p, width).expect("writing to a String can't fail");
    out
}

pub fn markdown(r: &Report) -> String {
    let mut out = String::new();
    write_markdown(&mut out, r).expect("writing to a String can't fail");
    out
}

pub fn compare(reports: &[Report], p: Palette) -> String {
    let mut out = String::new();
    write_compare(&mut out, reports, p).expect("writing to a String can't fail");
    out
}

fn write_text(out: &mut String, r: &Report, p: Palette, width: usize) -> fmt::Result {
    let (user, stats) = (&r.user, &r.stats);

    write!(out, "  {}", p.bold(&p.fg(ACCENT, &user.login)))?;
    if let Some(name) = user.name.as_deref().filter(|n| !n.is_empty()) {
        write!(out, "  {name}")?;
    }
    let kind = if user.is_org() { "org" } else { "user" };
    let since = user.created_at.get(..4).unwrap_or("?");
    writeln!(out)?;
    writeln!(
        out,
        "  {}\n",
        p.dim(&format!(
            "{kind} since {since}, {} followers, {} following",
            num(user.followers),
            num(user.following)
        ))
    )?;

    write!(
        out,
        "  {} repos   {} stars   forked {}",
        p.bold(&num(stats.total as u32)),
        p.bold(&p.fg(STAR, &num(stats.stars))),
        p.bold(&plural(stats.forks as usize, "time")),
    )?;
    if stats.archived > 0 {
        write!(out, "   {} archived", stats.archived)?;
    }
    writeln!(out)?;

    if stats.private > 0 || stats.fork_repos > 0 {
        let mut split = if stats.private > 0 {
            format!(
                "{} own ({} public, {} private)",
                stats.own, stats.public, stats.private
            )
        } else {
            format!("{} own", stats.own)
        };
        if stats.fork_repos > 0 {
            split += &format!(", {}", plural(stats.fork_repos, "fork"));
            if stats.repos < stats.total {
                split += " left out of the stats (--forks adds them)";
            }
        }
        writeln!(out, "  {}", p.dim(&split))?;
    }

    if let Some(repo) = &stats.last_push {
        let line = format!(
            "last push: {} on {}, {} active in the last 30 days",
            repo.name,
            date(&repo.pushed_at),
            plural(stats.active_last_30_days, "repo")
        );
        writeln!(out, "  {}", p.dim(&line))?;
    }
    if let Some(repo) = &stats.newest {
        let line = format!("newest: {}, created {}", repo.name, date(&repo.created_at));
        writeln!(out, "  {}", p.dim(&line))?;
    }

    if let (Some(c), Some(s)) = (&r.contributions, &r.streaks) {
        heatmap::render(out, c, s, p, width)?;
    }

    if !stats.languages.is_empty() {
        write_languages(out, &stats.languages, p)?;
    }

    if !stats.top.is_empty() {
        write_top(out, r, p, width)?;
    }

    if !r.has_token {
        let hint =
            "log in with `gh auth login` to also see contributions and code size per language";
        writeln!(out, "\n  {}", p.dim(hint))?;
    }
    Ok(())
}

fn write_languages(out: &mut String, langs: &[Language], p: Palette) -> fmt::Result {
    let by_size = langs[0].bytes.is_some();
    let title = if by_size {
        "by code size"
    } else {
        "by main language"
    };
    writeln!(out, "\n  {}  {}", p.bold("Languages"), p.dim(title))?;

    let shown = &langs[..langs.len().min(MAX_LANGUAGES)];
    let name_w = shown
        .iter()
        .map(|l| l.name.chars().count())
        .max()
        .unwrap_or(0);
    let max = shown[0].percent;
    for l in shown {
        let color = l
            .color
            .as_deref()
            .and_then(colors::hex)
            .unwrap_or_else(|| colors::language(&l.name));
        let bar = format!("{:<BAR_CELLS$}", bar(l.percent / max, BAR_CELLS));
        let detail = match l.bytes {
            Some(b) => size(b),
            None => plural(l.repos, "repo"),
        };
        writeln!(
            out,
            "  {} {:<name_w$}  {}  {:>5.1}%  {}",
            p.fg(color, "●"),
            l.name,
            p.fg(color, &bar),
            l.percent,
            p.dim(&detail),
        )?;
    }
    Ok(())
}

fn write_top(out: &mut String, r: &Report, p: Palette, width: usize) -> fmt::Result {
    let top = &r.stats.top;
    writeln!(out, "\n  {}", p.bold("Top repos"))?;
    let star_w = num(top[0].stargazers_count).len();
    let name_w = top
        .iter()
        .map(|r| r.name.chars().count())
        .max()
        .unwrap_or(0);
    let lang_w = top
        .iter()
        .map(|r| r.language.as_deref().unwrap_or("-").chars().count())
        .max()
        .unwrap_or(0);
    let used = 2 + 2 + star_w + 2 + name_w + 2 + lang_w + 2;

    for repo in top {
        let lang = repo.language.as_deref().unwrap_or("-");
        let mut line = format!(
            "  {} {:>star_w$}  {:<name_w$}  {}",
            p.fg(STAR, "★"),
            num(repo.stargazers_count),
            repo.name,
            p.fg(colors::language(lang), &format!("{lang:<lang_w$}")),
        );
        let mut desc = strip_shortcodes(repo.description.as_deref().unwrap_or(""));
        if repo.private {
            desc = format!("(private) {desc}").trim_end().to_string();
        }
        let room = width.saturating_sub(used);
        if !desc.is_empty() && room >= 12 {
            line += &format!("  {}", p.dim(&truncate(&desc, room)));
        }
        writeln!(out, "{}", line.trim_end())?;
    }
    Ok(())
}

fn write_markdown(out: &mut String, r: &Report) -> fmt::Result {
    let (user, stats) = (&r.user, &r.stats);
    let name = user
        .name
        .as_deref()
        .filter(|n| !n.is_empty())
        .unwrap_or(&user.login);
    writeln!(out, "### {name} ([@{}]({}))\n", user.login, user.html_url)?;
    writeln!(
        out,
        "**{}** repos · **{}** stars · forked **{}** times · **{}** followers\n",
        num(stats.total as u32),
        num(stats.stars),
        num(stats.forks),
        num(user.followers)
    )?;

    if let (Some(c), Some(s)) = (&r.contributions, &r.streaks) {
        writeln!(
            out,
            "**{}** contributions in the last year · longest streak **{}**\n",
            num(c.total),
            plural(s.longest as usize, "day")
        )?;
    }

    if !stats.languages.is_empty() {
        let by_size = stats.languages[0].bytes.is_some();
        if by_size {
            out.push_str("| Language | Share | Code |\n| --- | ---: | ---: |\n");
        } else {
            out.push_str("| Language | Share | Repos |\n| --- | ---: | ---: |\n");
        }
        for l in stats.languages.iter().take(MAX_LANGUAGES) {
            let detail = l.bytes.map_or_else(|| l.repos.to_string(), size);
            writeln!(out, "| {} | {:.1}% | {detail} |", l.name, l.percent)?;
        }
        out.push('\n');
    }

    if !stats.top.is_empty() {
        out.push_str("| Repo | Stars | Language | About |\n| --- | ---: | --- | --- |\n");
        for repo in &stats.top {
            let desc = repo.description.as_deref().unwrap_or("");
            writeln!(
                out,
                "| [{}]({}) | {} | {} | {} |",
                repo.name,
                repo.html_url,
                num(repo.stargazers_count),
                repo.language.as_deref().unwrap_or("-"),
                md_cell(&truncate(desc, 80)),
            )?;
        }
        out.push('\n');
    }

    out.push_str("<sub>made with [gh-stats](https://github.com/CanReader/gh-stats)</sub>\n");
    Ok(())
}

struct Row {
    label: &'static str,
    values: Vec<Option<u64>>,
    text: Vec<String>,
    highlight: bool,
}

fn write_compare(out: &mut String, reports: &[Report], p: Palette) -> fmt::Result {
    let counted = |label, f: &dyn Fn(&Report) -> Option<u64>| {
        let values: Vec<Option<u64>> = reports.iter().map(f).collect();
        let text = values
            .iter()
            .map(|v| v.map_or_else(|| "-".to_string(), |n| num(n as u32)))
            .collect();
        Row {
            label,
            values,
            text,
            highlight: true,
        }
    };
    let words = |label, f: &dyn Fn(&Report) -> String| Row {
        label,
        values: vec![None; reports.len()],
        text: reports.iter().map(f).collect(),
        highlight: false,
    };

    let rows = [
        counted("repos", &|r| Some(r.stats.total as u64)),
        counted("stars", &|r| Some(r.stats.stars.into())),
        counted("forked", &|r| Some(r.stats.forks.into())),
        counted("followers", &|r| Some(r.user.followers.into())),
        counted("contributions", &|r| {
            r.contributions.as_ref().map(|c| c.total.into())
        }),
        counted("current streak", &|r| {
            r.streaks.as_ref().map(|s| s.current.into())
        }),
        counted("longest streak", &|r| {
            r.streaks.as_ref().map(|s| s.longest.into())
        }),
        counted("active, 30 days", &|r| {
            Some(r.stats.active_last_30_days as u64)
        }),
        words("top language", &|r| {
            r.stats
                .languages
                .first()
                .map_or_else(|| "-".to_string(), |l| l.name.clone())
        }),
        words("on GitHub since", &|r| {
            r.user.created_at.get(..4).unwrap_or("-").to_string()
        }),
    ];

    let label_w = rows.iter().map(|r| r.label.len()).max().unwrap_or(0) + 2;
    let col_w: Vec<usize> = reports
        .iter()
        .enumerate()
        .map(|(i, r)| {
            rows.iter()
                .map(|row| row.text[i].chars().count())
                .chain([r.user.login.chars().count()])
                .max()
                .unwrap_or(0)
                + 3
        })
        .collect();

    write!(out, "  {:label_w$}", "")?;
    for (r, w) in reports.iter().zip(&col_w) {
        let name = format!("{:>w$}", r.user.login);
        write!(out, "{}", p.bold(&p.fg(ACCENT, &name)))?;
    }
    writeln!(out)?;

    for row in &rows {
        let best = row.values.iter().flatten().max().copied();
        let tied = row.values.iter().filter(|v| **v == best).count() > 1;
        write!(out, "  {}", p.dim(&format!("{:label_w$}", row.label)))?;
        for (i, w) in col_w.iter().enumerate() {
            let cell = format!("{:>w$}", row.text[i]);
            let wins =
                row.highlight && !tied && best.is_some_and(|b| b > 0) && row.values[i] == best;
            if wins {
                write!(out, "{}", p.bold(&p.fg(GREEN, &cell)))?;
            } else {
                write!(out, "{cell}")?;
            }
        }
        writeln!(out)?;
    }
    Ok(())
}

fn bar(ratio: f64, cells: usize) -> String {
    const PARTS: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
    if ratio.is_nan() || ratio <= 0.0 {
        return String::new();
    }
    let eighths = ((ratio.min(1.0) * cells as f64 * 8.0).round() as usize).max(1);
    let mut s = "█".repeat(eighths / 8);
    let rest = eighths % 8;
    if rest > 0 {
        s.push(PARTS[rest]);
    }
    s
}

fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn date(iso: &Option<String>) -> &str {
    iso.as_deref().and_then(|d| d.get(..10)).unwrap_or("?")
}

pub fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{} {word}s", num(n as u32))
    }
}

pub fn num(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

// GitHub shows :crab: as an emoji, a terminal can't
fn strip_shortcodes(s: &str) -> String {
    let is_shortcode = |w: &str| {
        w.strip_prefix(':')
            .and_then(|w| w.strip_suffix(':'))
            .is_some_and(|inner| {
                !inner.is_empty()
                    && inner
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_+-".contains(c))
            })
    };
    s.split_whitespace()
        .filter(|w| !is_shortcode(w))
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
    t.truncate(t.trim_end().len());
    t.push('…');
    t
}

fn md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{Repo, User};
    use crate::graphql::Contributions;
    use crate::stats::{
        collect, streaks,
        tests::{NOW, day, sample},
    };

    fn user(login: &str) -> User {
        User {
            login: login.into(),
            name: Some("Some One".into()),
            kind: "User".into(),
            html_url: format!("https://github.com/{login}"),
            public_repos: 5,
            followers: 12,
            following: 3,
            created_at: "2019-04-01T00:00:00Z".into(),
        }
    }

    fn report(repos: &[Repo], with_contrib: bool) -> Report {
        let contributions = with_contrib.then(|| Contributions {
            total: 40,
            commits: 30,
            pull_requests: 5,
            issues: 3,
            reviews: 2,
            private: 0,
            days: vec![day("2026-10-03", 2), day("2026-10-04", 5)],
        });
        Report {
            user: user("someone"),
            stats: collect(repos, false, 5, NOW, None),
            streaks: contributions.as_ref().map(|c| streaks(&c.days)),
            contributions,
            has_token: with_contrib,
        }
    }

    #[test]
    fn bars() {
        assert_eq!(bar(1.0, 4), "████");
        assert_eq!(bar(0.5, 4), "██");
        assert_eq!(bar(1.0 / 3.0, 4), "█▍");
        assert_eq!(bar(0.001, 4), "▏");
        assert_eq!(bar(0.0, 4), "");
        assert_eq!(bar(f64::NAN, 4), "");
    }

    #[test]
    fn sizes() {
        assert_eq!(size(900), "900 B");
        assert_eq!(size(2048), "2.0 KB");
        assert_eq!(size(5 * 1024 * 1024 + 300_000), "5.3 MB");
    }

    #[test]
    fn shortcodes() {
        assert_eq!(
            strip_shortcodes(":crab: Small exercises"),
            "Small exercises"
        );
        assert_eq!(strip_shortcodes("fast :zap: and  safe"), "fast and safe");
        assert_eq!(strip_shortcodes("ratio 1:2 is fine"), "ratio 1:2 is fine");
        assert_eq!(strip_shortcodes("::"), "::");
    }

    #[test]
    fn plurals() {
        assert_eq!(plural(1, "fork"), "1 fork");
        assert_eq!(plural(0, "fork"), "0 forks");
        assert_eq!(plural(68553, "time"), "68,553 times");
    }

    #[test]
    fn number_grouping() {
        assert_eq!(num(0), "0");
        assert_eq!(num(999), "999");
        assert_eq!(num(1000), "1,000");
        assert_eq!(num(400838), "400,838");
        assert_eq!(num(1234567), "1,234,567");
    }

    #[test]
    fn truncating() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("a long description here", 10), "a long de…");
        assert_eq!(truncate("çok güzel bir şey", 6), "çok g…");
    }

    #[test]
    fn plain_text_report() {
        let out = text(&report(&sample(), false), Palette::None, 80);
        assert!(!out.contains('\x1b'));
        assert!(out.contains("someone  Some One"));
        assert!(out.contains("user since 2019, 12 followers, 3 following"));
        assert!(out.contains("6 repos   13 stars   forked 5 times   1 archived"));
        assert!(out.contains("5 own (4 public, 1 private), 1 fork left out of the stats"));
        assert!(out.contains("last push: cli on 2026-10-02, 2 repos active in the last 30 days"));
        assert!(out.contains("newest: secret, created 2026-09-01"));
        assert!(out.contains("Languages  by main language"));
        assert!(out.contains("● Rust"));
        assert!(out.contains("75.0%  3 repos"));
        assert!(out.contains("★ 7  cli"));
        assert!(out.contains("Small command line tool"));
        assert!(out.contains("gh auth login"));
        assert!(!out.contains("Contributions"));
    }

    #[test]
    fn report_with_contributions() {
        let out = text(&report(&sample(), true), Palette::None, 80);
        assert!(out.contains("40 in the last year   30 commits"));
        assert!(out.contains("current streak 2 days"));
        assert!(!out.contains("gh auth login"));
    }

    #[test]
    fn narrow_terminal_drops_descriptions() {
        let out = text(&report(&sample(), false), Palette::None, 20);
        assert!(!out.contains("Small command"));
    }

    #[test]
    fn colored_report_has_escape_codes() {
        let out = text(&report(&sample(), false), Palette::TrueColor, 80);
        assert!(out.contains("\x1b[38;2;"));
    }

    #[test]
    fn markdown_report() {
        let mut repos = sample();
        repos[3].description = Some("pipes | in | text".into());
        repos.retain(|r| !r.private);
        let md = markdown(&report(&repos, true));
        assert!(md.starts_with("### Some One ([@someone](https://github.com/someone))"));
        assert!(md.contains("**5** repos · **13** stars · forked **4** times"));
        assert!(md.contains("**40** contributions in the last year · longest streak **2 days**"));
        assert!(md.contains("| Rust | 66.7% | 2 |"));
        assert!(md.contains(
            "| [cli](https://github.com/someone/cli) | 7 | Rust | pipes \\| in \\| text |"
        ));
    }

    #[test]
    fn compare_table() {
        let a = report(&sample(), true);
        let mut b = report(&sample()[..2], false);
        b.user = user("other");
        let out = compare(&[a, b], Palette::None);
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].contains("someone") && lines[0].contains("other"));
        let repos = lines.iter().find(|l| l.contains("repos")).unwrap();
        assert!(repos.contains('6') && repos.contains('2'));
        let contrib = lines.iter().find(|l| l.contains("contributions")).unwrap();
        assert!(contrib.contains("40") && contrib.contains('-'));
    }

    #[test]
    fn compare_highlights_winner() {
        let a = report(&sample(), true);
        let mut b = report(&sample()[..2], false);
        b.user = user("other");
        let out = compare(&[a, b], Palette::TrueColor);
        assert!(out.contains("\x1b[38;2;63;185;80m"));
    }
}
