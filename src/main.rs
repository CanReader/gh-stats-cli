mod cache;
mod colors;
mod github;
mod graphql;
mod heatmap;
mod render;
mod stats;
mod svg;

use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, ValueEnum};
use clap_complete::Shell;
use serde::Serialize;

use cache::Cache;
use colors::Palette;
use github::{Client, User};
use graphql::Contributions;
use stats::{Stats, Streaks};

const SVG_COLUMNS: usize = 118;

/// GitHub stats for any user or org, right in your terminal.
#[derive(Parser)]
#[command(name = "gh-stats", version)]
struct Args {
    /// GitHub usernames or orgs (defaults to you, if you're logged in with gh)
    users: Vec<String>,

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
    let users = if args.users.is_empty() {
        vec![client.me()?]
    } else {
        args.users.clone()
    };
    if args.compare && users.len() < 2 {
        bail!("--compare needs at least two users");
    }

    let mut reports = Vec::new();
    let mut all_ok = true;
    for login in &users {
        match load(&client, login, args) {
            Ok(r) => reports.push(r),
            Err(e) => {
                eprintln!("gh-stats: {e:#}");
                all_ok = false;
            }
        }
    }
    if reports.is_empty() {
        return Ok(false);
    }

    if args.json {
        let json = match reports.as_slice() {
            [one] => serde_json::to_string_pretty(one)?,
            many => serde_json::to_string_pretty(many)?,
        };
        println!("{json}");
        return Ok(all_ok);
    }
    if args.md {
        let blocks: Vec<String> = reports.iter().map(render::markdown).collect();
        print!("{}", blocks.join("\n"));
        return Ok(all_ok);
    }

    let (palette, width) = match &args.svg {
        Some(_) => (Palette::TrueColor, SVG_COLUMNS),
        None => (palette(args.color), terminal_width()),
    };
    let output = if args.compare {
        render::compare(&reports, palette)
    } else {
        let blocks: Vec<String> = reports
            .iter()
            .map(|r| render::text(r, palette, width))
            .collect();
        blocks.join("\n")
    };

    match &args.svg {
        Some(path) => {
            let mut title = format!("gh stats {}", users.join(" "));
            if args.compare {
                title += " --compare";
            }
            std::fs::write(path, svg::render(&output, &title))
                .with_context(|| format!("could not write {}", path.display()))?;
            eprintln!("saved {}", path.display());
        }
        None => print!("{output}"),
    }
    Ok(all_ok)
}

fn load(client: &Client, login: &str, args: &Args) -> Result<Report> {
    let user = client.user(login)?;
    // --md and --svg are meant to be shared, so they never show private repos
    let with_private = !(args.public || args.md || args.svg.is_some());
    let since = args.since.as_deref().unwrap_or("");

    let mut repos = client.repos(&user, with_private)?;
    repos
        .retain(|r| (with_private || !r.private) && r.created_at.as_deref().unwrap_or("") >= since);

    let mut sizes = None;
    let mut contributions = None;
    if client.has_token() {
        match graphql::languages(client, &user.login, with_private, args.forks) {
            Ok(mut s) => {
                s.retain(|r| r.created_at.as_str() >= since);
                sizes = Some(s);
            }
            Err(e) => eprintln!("gh-stats: no code sizes for {}: {e:#}", user.login),
        }
        if !user.is_org() {
            match graphql::contributions(client, &user.login) {
                Ok(mut c) => {
                    if !with_private {
                        c.private = 0;
                    }
                    contributions = Some(c);
                }
                Err(e) => eprintln!("gh-stats: no contributions for {}: {e:#}", user.login),
            }
        }
    }

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

fn palette(mode: ColorMode) -> Palette {
    let on = match mode {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
    };
    let truecolor =
        std::env::var("COLORTERM").is_ok_and(|v| v.contains("truecolor") || v.contains("24bit"));
    match (on, truecolor) {
        (false, _) => Palette::None,
        (true, true) => Palette::TrueColor,
        (true, false) => Palette::Ansi256,
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
    fn cli_is_valid() {
        Args::command().debug_assert();
    }

    #[test]
    fn compare_and_json_conflict() {
        let res = Args::try_parse_from(["gh-stats", "a", "b", "--compare", "--json"]);
        assert!(res.is_err());
    }
}
