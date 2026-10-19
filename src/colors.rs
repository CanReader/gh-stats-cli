// same colors GitHub uses, from github-linguist
const LANGUAGES: &[(&str, u32)] = &[
    ("Assembly", 0x6E4C13),
    ("C", 0x555555),
    ("C#", 0x178600),
    ("C++", 0xF34B7D),
    ("CMake", 0xDA3434),
    ("CSS", 0x663399),
    ("Clojure", 0xDB5855),
    ("Cuda", 0x3A4E3A),
    ("Dart", 0x00B4AB),
    ("Dockerfile", 0x384D54),
    ("Elixir", 0x6E4A7E),
    ("Erlang", 0xB83998),
    ("GLSL", 0x5686A5),
    ("Go", 0x00ADD8),
    ("HLSL", 0xAACE60),
    ("HTML", 0xE34C26),
    ("Haskell", 0x5E5086),
    ("Java", 0xB07219),
    ("JavaScript", 0xF1E05A),
    ("Julia", 0xA270BA),
    ("Jupyter Notebook", 0xDA5B0B),
    ("Kotlin", 0xA97BFF),
    ("Lua", 0x000080),
    ("Makefile", 0x427819),
    ("Nix", 0x7E7EFF),
    ("OCaml", 0xEF7A08),
    ("Objective-C", 0x438EFF),
    ("PHP", 0x4F5D95),
    ("Perl", 0x0298C3),
    ("PowerShell", 0x012456),
    ("Python", 0x3572A5),
    ("R", 0x198CE7),
    ("Ruby", 0x701516),
    ("Rust", 0xDEA584),
    ("Scala", 0xC22D40),
    ("Shell", 0x89E051),
    ("Svelte", 0xFF3E00),
    ("Swift", 0xF05138),
    ("TypeScript", 0x3178C6),
    ("Vim Script", 0x199F4B),
    ("Vue", 0x41B883),
    ("Zig", 0xEC915C),
];

const FALLBACK: u32 = 0x8B949E;

pub fn language(name: &str) -> Rgb {
    let hex = LANGUAGES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map_or(FALLBACK, |(_, c)| *c);
    ((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub fn hex(s: &str) -> Option<Rgb> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some(((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

pub type Rgb = (u8, u8, u8);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub fg: Rgb,
    pub dim: Rgb,
    pub bg: Rgb,
    pub border: Rgb,
    pub accent: Rgb,
    pub star: Rgb,
    pub green: Rgb,
    /// heatmap squares, from no contributions to the most
    pub cells: [Rgb; 5],
}

// GitHub's own dark and light colors, the empty dark square is a bit lighter
// than on github.com so it still shows on a black terminal
pub const DARK: Theme = Theme {
    fg: (0xC9, 0xD1, 0xD9),
    dim: (0x8B, 0x94, 0x9E),
    bg: (0x0D, 0x11, 0x17),
    border: (0x30, 0x36, 0x3D),
    accent: (0x58, 0xA6, 0xFF),
    star: (0xE3, 0xB3, 0x41),
    green: (0x3F, 0xB9, 0x50),
    cells: [
        (0x2D, 0x33, 0x3B),
        (0x0E, 0x44, 0x29),
        (0x00, 0x6D, 0x32),
        (0x26, 0xA6, 0x41),
        (0x39, 0xD3, 0x53),
    ],
};

pub const LIGHT: Theme = Theme {
    fg: (0x1F, 0x23, 0x28),
    dim: (0x59, 0x63, 0x6E),
    bg: (0xFF, 0xFF, 0xFF),
    border: (0xD0, 0xD7, 0xDE),
    accent: (0x09, 0x69, 0xDA),
    star: (0x9A, 0x67, 0x00),
    green: (0x1A, 0x7F, 0x37),
    cells: [
        (0xEB, 0xED, 0xF0),
        (0x9B, 0xE9, 0xA8),
        (0x40, 0xC4, 0x63),
        (0x30, 0xA1, 0x4E),
        (0x21, 0x6E, 0x39),
    ],
};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Plain,
    Ansi256,
    TrueColor,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    mode: Mode,
    pub theme: Theme,
}

impl Palette {
    pub fn plain() -> Self {
        Self {
            mode: Mode::Plain,
            theme: DARK,
        }
    }

    pub fn ansi256(theme: Theme) -> Self {
        Self {
            mode: Mode::Ansi256,
            theme,
        }
    }

    pub fn truecolor(theme: Theme) -> Self {
        Self {
            mode: Mode::TrueColor,
            theme,
        }
    }

    pub fn is_plain(self) -> bool {
        self.mode == Mode::Plain
    }

    pub fn fg(self, rgb: Rgb, text: &str) -> String {
        let (r, g, b) = rgb;
        match self.mode {
            Mode::Plain => text.to_string(),
            Mode::TrueColor => format!("\x1b[38;2;{r};{g};{b}m{text}\x1b[0m"),
            Mode::Ansi256 => format!("\x1b[38;5;{}m{text}\x1b[0m", to_256(rgb)),
        }
    }

    pub fn bold(self, text: &str) -> String {
        if self.is_plain() {
            text.to_string()
        } else {
            format!("\x1b[1m{text}\x1b[0m")
        }
    }

    pub fn dim(self, text: &str) -> String {
        if self.is_plain() {
            text.to_string()
        } else {
            format!("\x1b[2m{text}\x1b[0m")
        }
    }
}

fn to_256((r, g, b): Rgb) -> u8 {
    let level = |v: u8| ((v as u16 * 5 + 127) / 255) as u8;
    16 + 36 * level(r) + 6 * level(g) + level(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_and_unknown_languages() {
        assert_eq!(language("Rust"), (0xDE, 0xA5, 0x84));
        assert_eq!(language("rust"), language("Rust"));
        assert_eq!(language("SomethingNew"), (0x8B, 0x94, 0x9E));
    }

    #[test]
    fn hex_colors() {
        assert_eq!(hex("#dea584"), Some((0xDE, 0xA5, 0x84)));
        assert_eq!(hex("dea584"), None);
        assert_eq!(hex("#fff"), None);
        assert_eq!(hex("#zzzzzz"), None);
    }

    #[test]
    fn ansi_256_mapping() {
        assert_eq!(to_256((0, 0, 0)), 16);
        assert_eq!(to_256((255, 255, 255)), 231);
        assert_eq!(to_256((255, 0, 0)), 196);
    }

    #[test]
    fn no_color_is_plain() {
        assert_eq!(Palette::plain().fg((1, 2, 3), "x"), "x");
        assert_eq!(Palette::plain().bold("x"), "x");
        assert!(
            Palette::truecolor(DARK)
                .fg((1, 2, 3), "x")
                .contains("38;2;1;2;3")
        );
    }
}
