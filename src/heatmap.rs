use std::fmt::{self, Write};

use crate::colors::Palette;
use crate::graphql::{Contributions, Day};
use crate::render::{num, plural};
use crate::stats::{Streaks, days};

const PLAIN: [char; 5] = ['·', '░', '▒', '▓', '█'];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const LABEL_W: usize = 6;

pub fn render(
    out: &mut String,
    c: &Contributions,
    s: &Streaks,
    p: Palette,
    width: usize,
) -> fmt::Result {
    writeln!(out, "\n  {}", p.bold("Contributions"))?;
    writeln!(
        out,
        "  {} in {}   {}   {}   {}   {}",
        p.bold(&p.fg(p.theme.green, &num(c.total))),
        period(c),
        count(p, c.commits, "commit"),
        count(p, c.pull_requests, "pull request"),
        count(p, c.issues, "issue"),
        count(p, c.reviews, "review"),
    )?;

    // a current streak only makes sense when looking at the last 12 months
    let mut line = match c.year {
        None => format!(
            "current streak {}   longest {}",
            p.bold(&plural(s.current as usize, "day")),
            p.bold(&plural(s.longest as usize, "day")),
        ),
        Some(_) => format!(
            "longest streak {}",
            p.bold(&plural(s.longest as usize, "day"))
        ),
    };
    if let Some((date, n)) = &s.best_day {
        line += &format!("   best day {} on {}", p.bold(&num(*n)), short_date(date));
    }
    if c.private > 0 {
        line += &format!("   {} in private repos", num(c.private));
    }
    writeln!(out, "  {}", p.dim(&line))?;
    writeln!(out)?;

    let grid = Grid::new(&c.days, (width.saturating_sub(LABEL_W + 2)) / 2);
    if grid.cols == 0 {
        return Ok(());
    }

    writeln!(out, "{}{}", " ".repeat(LABEL_W), p.dim(&grid.month_row()))?;
    for (row, label) in ["", "Mon", "", "Wed", "", "Fri", ""].iter().enumerate() {
        let mut line = format!("  {label:<4}");
        for col in 0..grid.cols {
            if col > 0 {
                line.push(' ');
            }
            line += &match grid.cells[row][col] {
                Some(level) => cell(p, level),
                None => " ".to_string(),
            };
        }
        writeln!(
            out,
            "{}",
            p.dim(&line[..LABEL_W]).to_string() + &line[LABEL_W..]
        )?;
    }

    let legend: Vec<String> = (0..5).map(|l| cell(p, l)).collect();
    let legend = format!("{} {} {}", p.dim("Less"), legend.join(" "), p.dim("More"));
    // "Less ■ ■ ■ ■ ■ More" is 19 columns wide
    let pad = (LABEL_W + grid.cols * 2 - 1).saturating_sub(19);
    writeln!(out, "{}{legend}", " ".repeat(pad))
}

pub fn period(c: &Contributions) -> String {
    c.year
        .map_or_else(|| "the last year".to_string(), |y| y.to_string())
}

fn count(p: Palette, n: u32, word: &str) -> String {
    let text = plural(n as usize, word);
    match text.split_once(' ') {
        Some((n, rest)) => format!("{} {}", p.bold(n), rest),
        None => text,
    }
}

fn cell(p: Palette, level: u8) -> String {
    let level = level.min(4) as usize;
    if p.is_plain() {
        PLAIN[level].to_string()
    } else {
        p.fg(p.theme.cells[level], "■")
    }
}

fn short_date(date: &str) -> String {
    let month = date
        .get(5..7)
        .and_then(|m| m.parse::<usize>().ok())
        .and_then(|m| MONTHS.get(m.wrapping_sub(1)));
    let day = date.get(8..10).and_then(|d| d.parse::<u32>().ok());
    match (month, day) {
        (Some(m), Some(d)) => format!("{m} {d}"),
        _ => date.to_string(),
    }
}

struct Grid {
    cells: [Vec<Option<u8>>; 7],
    starts: Vec<String>,
    cols: usize,
}

impl Grid {
    fn new(days_in: &[Day], max_cols: usize) -> Self {
        let mut grid = Grid {
            cells: Default::default(),
            starts: Vec::new(),
            cols: 0,
        };
        let Some(first) = days_in.first().and_then(|d| days(&d.date)) else {
            return grid;
        };
        let offset = weekday(first);

        let mut placed = Vec::with_capacity(days_in.len());
        for d in days_in {
            let Some(n) = days(&d.date) else { continue };
            let i = (n - first) as usize + offset;
            placed.push((i / 7, i % 7, d));
        }
        let total_cols = placed.last().map_or(0, |&(c, _, _)| c + 1);
        let skip = total_cols.saturating_sub(max_cols);
        grid.cols = total_cols - skip;
        for row in &mut grid.cells {
            *row = vec![None; grid.cols];
        }
        grid.starts = vec![String::new(); grid.cols];

        for (col, row, d) in placed {
            let Some(col) = col.checked_sub(skip) else {
                continue;
            };
            grid.cells[row][col] = Some(d.level);
            if grid.starts[col].is_empty() {
                grid.starts[col] = d.date.clone();
            }
        }
        grid
    }

    fn month_row(&self) -> String {
        let mut row = String::new();
        let mut last_month = "";
        for (col, start) in self.starts.iter().enumerate() {
            let month = start.get(5..7).unwrap_or("");
            if month.is_empty() || month == last_month {
                continue;
            }
            last_month = month;
            let pos = col * 2;
            if pos < row.chars().count() + 1 && !row.is_empty() {
                continue;
            }
            let name = short_date(start);
            let name = name.split(' ').next().unwrap_or("");
            row += &" ".repeat(pos - row.chars().count());
            row += name;
        }
        row
    }
}

fn weekday(days_since_epoch: i64) -> usize {
    // 1970-01-01 was a Thursday, Sunday is 0
    (days_since_epoch + 4).rem_euclid(7) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::tests::day;

    fn sample() -> Contributions {
        // 2026-09-06 is a Sunday
        let start = days("2026-09-06").unwrap();
        let list = (0..35)
            .map(|i| day(&days_to_date(start + i), (i % 5) as u32))
            .collect();
        Contributions {
            total: 70,
            commits: 50,
            pull_requests: 1,
            issues: 4,
            reviews: 0,
            private: 3,
            year: None,
            days: list,
        }
    }

    fn days_to_date(n: i64) -> String {
        let (mut y, mut left) = (1970, n);
        loop {
            let len = if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                366
            } else {
                365
            };
            if left < len {
                break;
            }
            left -= len;
            y += 1;
        }
        let mut m = 1;
        loop {
            let first = days(&format!("{y}-{m:02}-01")).unwrap();
            let next = match m {
                12 => days(&format!("{}-01-01", y + 1)).unwrap(),
                _ => days(&format!("{y}-{:02}-01", m + 1)).unwrap(),
            };
            if n < next {
                return format!("{y}-{m:02}-{:02}", n - first + 1);
            }
            m += 1;
        }
    }

    #[test]
    fn weekdays() {
        assert_eq!(weekday(days("1970-01-01").unwrap()), 4);
        assert_eq!(weekday(days("2026-09-06").unwrap()), 0);
        assert_eq!(weekday(days("2026-10-10").unwrap()), 6);
    }

    #[test]
    fn grid_layout() {
        let c = sample();
        let g = Grid::new(&c.days, 100);
        assert_eq!(g.cols, 5);
        assert_eq!(g.cells[0][0], Some(0));
        assert_eq!(g.cells[1][0], Some(1));
        assert_eq!(g.cells[6][4], Some(4));
        assert_eq!(g.starts[0], "2026-09-06");
    }

    #[test]
    fn grid_keeps_latest_weeks_when_narrow() {
        let c = sample();
        let g = Grid::new(&c.days, 2);
        assert_eq!(g.cols, 2);
        assert_eq!(g.starts[0], "2026-09-27");
    }

    #[test]
    fn partial_first_week() {
        let days = [
            day("2026-10-08", 1),
            day("2026-10-09", 2),
            day("2026-10-11", 3),
        ];
        let g = Grid::new(&days, 10);
        assert_eq!(g.cols, 2);
        assert_eq!(g.cells[0][0], None);
        assert_eq!(g.cells[4][0], Some(1));
        assert_eq!(g.cells[0][1], Some(3));
    }

    #[test]
    fn month_labels() {
        let c = sample();
        let g = Grid::new(&c.days, 100);
        assert_eq!(g.month_row(), "Sep     Oct");
    }

    #[test]
    fn past_year_header() {
        let mut c = sample();
        c.year = Some(2024);
        let s = crate::stats::streaks(&c.days);
        let mut out = String::new();
        render(&mut out, &c, &s, Palette::plain(), 80).unwrap();
        assert!(out.contains("70 in 2024"));
        assert!(out.contains("longest streak"));
        assert!(!out.contains("current streak"));
    }

    #[test]
    fn dates() {
        assert_eq!(short_date("2026-03-04"), "Mar 4");
        assert_eq!(short_date("nope"), "nope");
    }

    #[test]
    fn plain_render() {
        let c = sample();
        let s = crate::stats::streaks(&c.days);
        let mut out = String::new();
        render(&mut out, &c, &s, Palette::plain(), 80).unwrap();
        assert!(
            out.contains(
                "70 in the last year   50 commits   1 pull request   4 issues   0 reviews"
            )
        );
        assert!(out.contains("3 in private repos"));
        assert!(out.contains("Mon"));
        assert!(out.contains("Less · ░ ▒ ▓ █ More"));
        assert!(!out.contains('\x1b'));
    }
}
