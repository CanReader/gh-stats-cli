mod cache;
mod colors;
mod github;
mod graphql;
mod heatmap;
mod render;
mod repo;
mod stats;
mod svg;

use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, ValueEnum};
use clap_complete::Shell;
use serde::Serialize;

use cache::Cache;
use colors::Palette;
use github::{Client, User};
use graphql::Contributions;
use repo::RepoReport;
use stats::{Stats, Streaks};

const SVG_COLUMNS: usize = 118;
const FIRST_YEAR: i32 = 2008;

/// GitHub stats for any user, org or repo, right in your terminal.
#[derive(Parser)]
#[command(name = "gh-stats", version)]
struct Args {
    /// Usernames, orgs or owner/repo (defaults to you, if you're logged in with gh)
    targets: Vec<String>,

    /// How many repos to list under "Top repos" (0 hides them)
    #[arg(short, long, default_value_t = 5)]
    top: usize,

    /// Count forked repos too
    #[arg(long)]
    forks: bool,

    /// Leave out private repos
    #[arg(long)]
    public: bool,

    /// Only count repos created on or after this date (2025, 2025-06 or 2025-06-01)
    #[arg(long, value_name = "DATE", value_parser = parse_since)]
    since: Option<String>,

    /// Show contributions for a past year instead of the last 12 months
    #[arg(long, value_parser = parse_year)]
    year: Option<i32>,

    /// Put two or more users side by side
    #[arg(long, conflicts_with_all = ["json", "md"])]
    compare: bool,

    /// Print JSON
    #[arg(long, conflicts_with = "md")]
    json: bool,

    /// Print Markdown, ready to paste into a profile README (public repos only)
    #[arg(long)]
    md: bool,

    /// Save the output as an SVG card instead of printing it (public repos only)
    #[arg(long, value_name = "FILE", conflicts_with_all = ["json", "md"])]
    svg: Option<PathBuf>,

    /// Colors for the SVG card, auto follows the viewer's light or dark mode
    #[arg(long, value_enum, default_value_t = ThemeArg::Auto)]
    theme: ThemeArg,

    /// Skip the 10 minute cache and ask GitHub again
    #[arg(long)]
    refresh: bool,

    /// When to use colors
    #[arg(long, value_enum, default_value_t = ColorMode::Auto)]
    color: ColorMode,

    /// Print a completion script for your shell
    #[arg(long, value_name = "SHELL", exclusive = true)]
    completions: Option<Shell>,
}

#[derive(Clone, Copy, ValueEnum)]
enum ColorMode {
    Auto,
    Always,
    Never,
}

#[derive(Clone, Copy, ValueEnum)]
enum ThemeArg {
    Auto,
    Dark,
    Light,
}

#[derive(Serialize)]
pub struct Report {
    pub user: User,
    #[serde(flatten)]
    pub stats: Stats,
    pub contributions: Option<Contributions>,
    pub streaks: Option<Streaks>,
    #[serde(skip)]
    pub has_token: bool,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Target {
    User(Box<Report>),
    Repo(Box<RepoReport>),
}

impl Args {
    // --md and --svg are meant to be shared, so they never show private repos
    fn shared(&self) -> bool {
        self.md || self.svg.is_some()
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    if let Some(shell) = args.completions {
        clap_complete::generate(shell, &mut Args::command(), "gh-stats", &mut io::stdout());
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("gh-stats: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<bool> {
    let client = Client::new(github::find_token(), Cache::new(!args.refresh));
    let names = if args.targets.is_empty() {
        vec![client.me()?]
    } else {
        args.targets.clone()
    };
    if args.compare {
        if names.len() < 2 {
            bail!("--compare needs at least two users");
        }
        if names.iter().any(|n| repo::parse(n).is_some()) {
            bail!("--compare only works with users and orgs");
        }
    }

    let results: Vec<Result<Target>> = thread::scope(|s| {
        let handles: Vec<_> = names
            .iter()
            .map(|name| s.spawn(|| load(&client, name, args)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("thread panicked"))
            .collect()
    });

    let mut targets = Vec::new();
    let mut all_ok = true;
    for result in results {
        match result {
            Ok(t) => targets.push(t),
            Err(e) => {
                eprintln!("gh-stats: {e:#}");
                all_ok = false;
            }
        }
    }
    if targets.is_empty() {
        return Ok(false);
    }

    if args.json {
        let json = match targets.as_slice() {
            [one] => serde_json::to_string_pretty(one)?,
            many => serde_json::to_string_pretty(many)?,
        };
        println!("{json}");
        return Ok(all_ok);
    }
    if args.md {
        let blocks: Vec<String> = targets
            .iter()
            .map(|t| match t {
                Target::User(r) => render::markdown(r),
                Target::Repo(r) => repo::markdown(r),
            })
            .collect();
        print!("{}", blocks.join("\n"));
        return Ok(all_ok);
    }

    // the svg swaps in the light colors itself, so it always gets the dark ones
    let (palette, width) = match &args.svg {
        Some(_) => (Palette::truecolor(colors::DARK), SVG_COLUMNS),
        None => (palette(args.color, args.theme), terminal_width()),
    };
    let output = if args.compare {
        let reports: Vec<Report> = targets
            .into_iter()
            .filter_map(|t| match t {
                Target::User(r) => Some(*r),
                Target::Repo(_) => None,
            })
            .collect();
        render::compare(&reports, palette)
    } else {
        let blocks: Vec<String> = targets
            .iter()
            .map(|t| match t {
                Target::User(r) => render::text(r, palette, width),
                Target::Repo(r) => repo::text(r, palette, width),
            })
            .collect();
        blocks.join("\n")
    };

    match &args.svg {
        Some(path) => {
            let mut title = format!("gh stats {}", names.join(" "));
            if args.compare {
                title += " --compare";
            }
            if let Some(year) = args.year {
                title += &format!(" --year {year}");
            }
            let scheme = match args.theme {
                ThemeArg::Auto => svg::Scheme::Auto,
                ThemeArg::Dark => svg::Scheme::Dark,
                ThemeArg::Light => svg::Scheme::Light,
            };
            std::fs::write(path, svg::render(&output, &title, scheme))
                .with_context(|| format!("could not write {}", path.display()))?;
            eprintln!("saved {}", path.display());
        }
        None => print!("{output}"),
    }
    Ok(all_ok)
}

fn load(client: &Client, name: &str, args: &Args) -> Result<Target> {
    if let Some((owner, repo_name)) = repo::parse(name) {
        let report = repo::load(client, owner, repo_name)?;
        if args.shared() {
            repo::ensure_shareable(&report)?;
        }
        return Ok(Target::Repo(Box::new(report)));
    }
    load_user(client, name, args).map(|r| Target::User(Box::new(r)))
}

fn load_user(client: &Client, login: &str, args: &Args) -> Result<Report> {
    let user = client.user(login)?;
    let with_private = !(args.public || args.shared());
    let since = args.since.as_deref().unwrap_or("");
    let graphql = client.has_token();

    // the three parts don't depend on each other, so ask for them at once
    let (repos, sizes, contributions) = thread::scope(|s| {
        let sizes = s.spawn(|| {
            graphql.then(|| graphql::languages(client, &user.login, with_private, args.forks))
        });
        let contributions = s.spawn(|| {
            (graphql && !user.is_org())
                .then(|| graphql::contributions(client, &user.login, args.year))
        });
        let repos = client.repos(&user, with_private);
        (
            repos,
            sizes.join().expect("thread panicked"),
            contributions.join().expect("thread panicked"),
        )
    });

    let mut repos = repos?;
    repos
        .retain(|r| (with_private || !r.private) && r.created_at.as_deref().unwrap_or("") >= since);

    // without code sizes or contributions the report still works, just shows less
    let sizes = sizes.and_then(|s| {
        s.inspect_err(|e| eprintln!("gh-stats: no code sizes for {}: {e:#}", user.login))
            .ok()
            .map(|mut s| {
                s.retain(|r| r.created_at.as_str() >= since);
                s
            })
    });
    let contributions = contributions.and_then(|c| {
        c.inspect_err(|e| eprintln!("gh-stats: no contributions for {}: {e:#}", user.login))
            .ok()
            .map(|mut c| {
                if !with_private {
                    c.private = 0;
                }
                c
            })
    });

    let stats = stats::collect(
        &repos,
        args.forks,
        args.top,
        github::now_secs(),
        sizes.as_deref(),
    );
    Ok(Report {
        streaks: contributions.as_ref().map(|c| stats::streaks(&c.days)),
        user,
        stats,
        contributions,
        has_token: client.has_token(),
    })
}

fn parse_since(s: &str) -> Result<String, String> {
    let ok = match s.len() {
        4 => s.chars().all(|c| c.is_ascii_digit()),
        7 | 10 => s.char_indices().all(|(i, c)| match i {
            4 | 7 => c == '-',
            _ => c.is_ascii_digit(),
        }),
        _ => false,
    };
    if ok {
        Ok(s.to_string())
    } else {
        Err("use a year, a month or a day, like 2025, 2025-06 or 2025-06-01".into())
    }
}

fn parse_year(s: &str) -> Result<i32, String> {
    let this_year = 1970 + (github::now_secs() / 31_556_952) as i32;
    match s.parse::<i32>() {
        Ok(y) if (FIRST_YEAR..=this_year).contains(&y) => Ok(y),
        _ => Err(format!("use a year between {FIRST_YEAR} and {this_year}")),
    }
}

fn palette(mode: ColorMode, theme: ThemeArg) -> Palette {
    let on = match mode {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
    };
    // a terminal can't tell us its background, so auto means dark here
    let theme = match theme {
        ThemeArg::Light => colors::LIGHT,
        ThemeArg::Auto | ThemeArg::Dark => colors::DARK,
    };
    let truecolor =
        std::env::var("COLORTERM").is_ok_and(|v| v.contains("truecolor") || v.contains("24bit"));
    match (on, truecolor) {
        (false, _) => Palette::plain(),
        (true, true) => Palette::truecolor(theme),
        (true, false) => Palette::ansi256(theme),
    }
}

fn terminal_width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|c| c.parse().ok())
        .unwrap_or(100)
        .clamp(40, 200)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_formats() {
        for ok in ["2025", "2025-06", "2025-06-01"] {
            assert!(parse_since(ok).is_ok(), "{ok}");
        }
        for bad in ["25", "2025/06", "2025-6-1", "june", "2025-06-01T00"] {
            assert!(parse_since(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn years() {
        assert_eq!(parse_year("2024"), Ok(2024));
        assert_eq!(parse_year("2008"), Ok(2008));
        for bad in ["2007", "3000", "24", "last"] {
            assert!(parse_year(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn cli_is_valid() {
        Args::command().debug_assert();
    }

    #[test]
    fn compare_and_json_conflict() {
        let res = Args::try_parse_from(["gh-stats", "a", "b", "--compare", "--json"]);
        assert!(res.is_err());
    }

    #[test]
    fn shared_outputs() {
        let md = Args::try_parse_from(["gh-stats", "a", "--md"]).unwrap();
        let svg = Args::try_parse_from(["gh-stats", "a", "--svg", "x.svg"]).unwrap();
        let plain = Args::try_parse_from(["gh-stats", "a"]).unwrap();
        assert!(md.shared() && svg.shared() && !plain.shared());
    }
}
