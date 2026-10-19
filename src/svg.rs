use std::fmt::{self, Write};

const CW: f64 = 8.4;
const LINE_H: f64 = 20.0;
const FONT_SIZE: f64 = 14.0;
const PAD: f64 = 24.0;
const TOP: f64 = 64.0;
const BG: &str = "#0d1117";
const BORDER: &str = "#30363d";
const FG: (u8, u8, u8) = (0xC9, 0xD1, 0xD9);
const DIM: (u8, u8, u8) = (0x8B, 0x94, 0x9E);

#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Style {
    fg: Option<(u8, u8, u8)>,
    bold: bool,
    dim: bool,
}

impl Style {
    fn color(self) -> (u8, u8, u8) {
        match (self.fg, self.dim) {
            (Some(c), _) => c,
            (None, true) => DIM,
            (None, false) => FG,
        }
    }
}

pub fn render(ansi: &str, title: &str) -> String {
    let mut svg = String::new();
    write_svg(&mut svg, ansi, title).expect("writing to a String can't fail");
    svg
}

fn write_svg(svg: &mut String, ansi: &str, title: &str) -> fmt::Result {
    let lines: Vec<Vec<(char, Style)>> = ansi.lines().map(parse_line).collect();
    // heatmap rows get the same spacing both ways so the squares sit on a grid
    let heights: Vec<f64> = lines
        .iter()
        .map(|l| {
            if l.iter().any(|(c, _)| *c == '■') {
                CW * 2.0
            } else {
                LINE_H
            }
        })
        .collect();

    let cols = lines
        .iter()
        .map(|l| {
            l.iter()
                .rposition(|(c, _)| !c.is_whitespace())
                .map_or(0, |i| i + 1)
        })
        .max()
        .unwrap_or(0)
        .max(title.chars().count() + 2);
    let width = (cols as f64 * CW + PAD * 2.0).ceil();
    let height = (TOP + heights.iter().sum::<f64>() + PAD).ceil();

    writeln!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">"#
    )?;
    writeln!(
        svg,
        r#"<rect x="0.5" y="0.5" width="{}" height="{}" rx="12" fill="{BG}" stroke="{BORDER}"/>"#,
        width - 1.0,
        height - 1.0
    )?;
    for (x, color) in [(24, "#ff5f57"), (44, "#febc2e"), (64, "#28c840")] {
        writeln!(svg, r#"<circle cx="{x}" cy="22" r="6" fill="{color}"/>"#)?;
    }
    writeln!(
        svg,
        r#"<g font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,'DejaVu Sans Mono',monospace" font-size="{FONT_SIZE}" xml:space="preserve">"#
    )?;
    writeln!(
        svg,
        r##"<text x="{PAD}" y="48"><tspan fill="#3fb950" font-weight="600">$ </tspan><tspan fill="{}">{}</tspan></text>"##,
        rgb(FG),
        escape(title)
    )?;

    let mut y = TOP;
    for (line, h) in lines.iter().zip(&heights) {
        draw_line(svg, line, y, *h)?;
        y += h;
    }
    svg.push_str("</g>\n</svg>\n");
    Ok(())
}

struct Run {
    start: usize,
    style: Style,
    text: String,
}

impl Run {
    fn write(&self, out: &mut String) -> fmt::Result {
        if self.text.trim().is_empty() {
            return Ok(());
        }
        let weight = if self.style.bold {
            r#" font-weight="600""#
        } else {
            ""
        };
        write!(
            out,
            r#"<tspan x="{:.1}" fill="{}"{weight}>{}</tspan>"#,
            PAD + self.start as f64 * CW,
            rgb(self.style.color()),
            escape(&self.text)
        )
    }
}

fn draw_line(svg: &mut String, line: &[(char, Style)], top: f64, h: f64) -> fmt::Result {
    let mid = top + h / 2.0;
    let mut text = String::new();
    let mut run: Option<Run> = None;
    let mut col = 0;

    while col < line.len() {
        let (c, style) = line[col];
        let x = PAD + col as f64 * CW;
        let fill = rgb(style.color());
        let is_shape = c == '■' || c == '●' || block_width(c).is_some();

        if (is_shape || run.as_ref().is_some_and(|r| r.style != style))
            && let Some(r) = run.take()
        {
            r.write(&mut text)?;
        }

        match c {
            '■' => {
                let s = 12.0;
                writeln!(
                    svg,
                    r#"<rect x="{:.1}" y="{:.1}" width="{s}" height="{s}" rx="2.5" fill="{fill}"/>"#,
                    x + (CW - s) / 2.0,
                    mid - s / 2.0
                )?;
                col += 1;
            }
            '●' => {
                writeln!(
                    svg,
                    r#"<circle cx="{:.1}" cy="{mid:.1}" r="4.5" fill="{fill}"/>"#,
                    x + CW / 2.0
                )?;
                col += 1;
            }
            _ if block_width(c).is_some() => {
                let mut w = 0.0;
                while let Some(part) = line
                    .get(col)
                    .filter(|(_, s)| *s == style)
                    .and_then(|(c, _)| block_width(*c))
                {
                    w += part * CW;
                    col += 1;
                }
                writeln!(
                    svg,
                    r#"<rect x="{x:.1}" y="{:.1}" width="{w:.1}" height="10" rx="3" fill="{fill}"/>"#,
                    mid - 5.0
                )?;
            }
            _ => {
                run.get_or_insert_with(|| Run {
                    start: col,
                    style,
                    text: String::new(),
                })
                .text
                .push(c);
                col += 1;
            }
        }
    }
    if let Some(r) = run {
        r.write(&mut text)?;
    }

    if !text.is_empty() {
        writeln!(
            svg,
            r#"<text y="{:.1}">{text}</text>"#,
            mid + FONT_SIZE * 0.35
        )?;
    }
    Ok(())
}

fn block_width(c: char) -> Option<f64> {
    match c {
        '█' => Some(1.0),
        '▉' => Some(7.0 / 8.0),
        '▊' => Some(6.0 / 8.0),
        '▋' => Some(5.0 / 8.0),
        '▌' => Some(4.0 / 8.0),
        '▍' => Some(3.0 / 8.0),
        '▎' => Some(2.0 / 8.0),
        '▏' => Some(1.0 / 8.0),
        _ => None,
    }
}

// only knows the escape codes gh-stats itself writes
fn parse_line(line: &str) -> Vec<(char, Style)> {
    let mut out = Vec::new();
    let mut style = Style::default();
    let mut rest = line;
    while let Some(start) = rest.find("\x1b[") {
        out.extend(rest[..start].chars().map(|c| (c, style)));
        let after = &rest[start + 2..];
        let Some(end) = after.find('m') else {
            rest = "";
            break;
        };
        let codes: Vec<u16> = after[..end]
            .split(';')
            .filter_map(|c| c.parse().ok())
            .collect();
        apply(&mut style, &codes);
        rest = &after[end + 1..];
    }
    out.extend(rest.chars().map(|c| (c, style)));
    out
}

fn apply(style: &mut Style, codes: &[u16]) {
    let mut i = 0;
    while i < codes.len() {
        match codes[i] {
            0 => *style = Style::default(),
            1 => style.bold = true,
            2 => style.dim = true,
            38 if codes.get(i + 1) == Some(&2) && i + 4 < codes.len() => {
                let c = |j: usize| codes[i + j] as u8;
                style.fg = Some((c(2), c(3), c(4)));
                i += 4;
            }
            _ => {}
        }
        i += 1;
    }
}

fn rgb((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_escape_codes() {
        let line = parse_line("a\x1b[1m\x1b[38;2;1;2;3mb\x1b[0mc");
        assert_eq!(line.len(), 3);
        assert_eq!(line[0].1, Style::default());
        assert_eq!(line[1].1.fg, Some((1, 2, 3)));
        assert!(line[1].1.bold);
        assert_eq!(line[2].1, Style::default());
    }

    #[test]
    fn shapes_for_squares_bars_and_dots() {
        let svg = render("x \x1b[38;2;57;211;83m■\x1b[0m\n● ███▌ 50%", "gh stats me");
        assert!(svg.contains(r##"fill="#39d353""##));
        assert!(svg.contains(r#"rx="2.5""#));
        assert!(svg.contains("<circle cx="));
        // 3.5 cells of bar
        assert!(svg.contains(r#"width="29.4" height="10""#));
        assert!(svg.contains("50%"));
        assert!(svg.contains("gh stats me"));
    }

    #[test]
    fn escapes_text() {
        let svg = render("a <b> & c", "t");
        assert!(svg.contains("a &lt;b&gt; &amp; c"));
    }

    #[test]
    fn plain_text_has_no_shapes() {
        let svg = render("just text", "t");
        assert!(!svg.contains("height=\"10\""));
        assert_eq!(svg.matches("<circle").count(), 3);
    }
}
