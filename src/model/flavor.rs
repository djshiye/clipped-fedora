//! What a text clip looks like it is, for the icon and caption in the list
//! and the tray: a link, an email address, a colour, code, or plain text.

use super::ClipKind;

#[derive(Debug, Clone, PartialEq)]
pub enum Flavor {
    Plain,
    /// `domain` is shown as the caption.
    Link {
        domain: String,
    },
    Email,
    /// RGBA, 0.0–1.0, for a swatch.
    Color([f32; 4]),
    Code,
    Files {
        count: usize,
    },
    Image,
}

impl Flavor {
    pub fn of(kind: ClipKind, text: Option<&str>) -> Self {
        match kind {
            ClipKind::Image => Flavor::Image,
            ClipKind::Files => Flavor::Files {
                count: text.map_or(0, |t| t.lines().filter(|l| !l.is_empty()).count()),
            },
            ClipKind::Text => classify(text.unwrap_or_default()),
        }
    }

    /// Symbolic icon for the row and the tray menu.
    pub fn icon_name(&self) -> &'static str {
        match self {
            Flavor::Plain => "text-x-generic-symbolic",
            Flavor::Link { .. } => "insert-link-symbolic",
            Flavor::Email => "mail-unread-symbolic",
            Flavor::Color(_) => "color-select-symbolic",
            Flavor::Code => "utilities-terminal-symbolic",
            Flavor::Files { .. } => "folder-symbolic",
            Flavor::Image => "image-x-generic-symbolic",
        }
    }
}

pub fn classify(text: &str) -> Flavor {
    let t = text.trim();
    let single_line = !t.contains('\n');
    if single_line && !t.contains(' ') {
        if let Some(domain) = link_domain(t) {
            return Flavor::Link { domain };
        }
        if is_email(t) {
            return Flavor::Email;
        }
        if let Some(rgba) = parse_hex_color(t) {
            return Flavor::Color(rgba);
        }
    }
    if looks_like_code(text) {
        return Flavor::Code;
    }
    Flavor::Plain
}

fn link_domain(t: &str) -> Option<String> {
    let rest = t
        .strip_prefix("https://")
        .or_else(|| t.strip_prefix("http://"))
        .or_else(|| t.strip_prefix("ftp://"))?;
    let host = rest.split(['/', '?', '#']).next()?;
    // Drop credentials and port; keep the name people recognise.
    let host = host.rsplit('@').next()?.split(':').next()?;
    let host = host.strip_prefix("www.").unwrap_or(host);
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

fn is_email(t: &str) -> bool {
    let t = t.strip_prefix("mailto:").unwrap_or(t);
    let Some((user, domain)) = t.split_once('@') else {
        return false;
    };
    !user.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains('@')
}

/// `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.
pub fn parse_hex_color(t: &str) -> Option<[f32; 4]> {
    let hex = t.strip_prefix('#')?;
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let nibble = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok().map(|v| v * 17);
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    let [r, g, b, a] = match hex.len() {
        3 => [nibble(0)?, nibble(1)?, nibble(2)?, 255],
        4 => [nibble(0)?, nibble(1)?, nibble(2)?, nibble(3)?],
        6 => [byte(0)?, byte(2)?, byte(4)?, 255],
        8 => [byte(0)?, byte(2)?, byte(4)?, byte(6)?],
        _ => return None,
    };
    let f = |v: u8| f32::from(v) / 255.0;
    Some([f(r), f(g), f(b), f(a)])
}

/// Cheap heuristic for showing text in a monospace font.
pub fn looks_like_code(text: &str) -> bool {
    let sample: String = text.chars().take(2000).collect();
    let braces = sample.matches(['{', '}', ';']).count();
    let indented = sample
        .lines()
        .filter(|l| l.starts_with("    ") || l.starts_with('\t'))
        .count();
    let tags = sample.matches("</").count();
    sample.starts_with("#!")
        || sample.starts_with("$ ")
        || braces >= 3
        || tags >= 2
        || (indented >= 2 && sample.lines().count() >= 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_clips() {
        assert_eq!(
            classify("https://www.Example.com:8080/a?b=c"),
            Flavor::Link {
                domain: "example.com".into()
            }
        );
        assert_eq!(classify("me@example.org"), Flavor::Email);
        assert_eq!(
            classify("#3584e4"),
            Flavor::Color([
                0x35 as f32 / 255.0,
                0x84 as f32 / 255.0,
                0xe4 as f32 / 255.0,
                1.0
            ])
        );
        assert_eq!(classify("#fff").icon_name(), "color-select-symbolic");
        assert_eq!(classify("fn a() { b(); c(); }"), Flavor::Code);
        assert_eq!(classify("hello there"), Flavor::Plain);
        assert_eq!(classify("#hashtag"), Flavor::Plain);
        assert_eq!(classify("see https://a.b later"), Flavor::Plain);
        assert_eq!(
            Flavor::of(ClipKind::Files, Some("file:///a\nfile:///b")),
            Flavor::Files { count: 2 }
        );
    }
}
