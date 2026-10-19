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

pub fn language(name: &str) -> (u8, u8, u8) {
    let hex = LANGUAGES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map_or(FALLBACK, |(_, c)| *c);
    ((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub fn hex(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some(((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Palette {
    None,
    Ansi256,
    TrueColor,
}

impl Palette {
    pub fn fg(self, rgb: (u8, u8, u8), text: &str) -> String {
        let (r, g, b) = rgb;
        match self {
            Palette::None => text.to_string(),
            Palette::TrueColor => format!("\x1b[38;2;{r};{g};{b}m{text}\x1b[0m"),
            Palette::Ansi256 => format!("\x1b[38;5;{}m{text}\x1b[0m", to_256(rgb)),
        }
    }

    pub fn bold(self, text: &str) -> String {
        match self {
            Palette::None => text.to_string(),
            _ => format!("\x1b[1m{text}\x1b[0m"),
        }
    }

    pub fn dim(self, text: &str) -> String {
        match self {
            Palette::None => text.to_string(),
            _ => format!("\x1b[2m{text}\x1b[0m"),
        }
    }
}

fn to_256((r, g, b): (u8, u8, u8)) -> u8 {
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
        assert_eq!(Palette::None.fg((1, 2, 3), "x"), "x");
        assert_eq!(Palette::None.bold("x"), "x");
        assert!(Palette::TrueColor.fg((1, 2, 3), "x").contains("38;2;1;2;3"));
    }
}
