//! Markdown → chat lines (010).
//!
//! The model writes markdown whether or not anything renders it, so the chat pane renders it:
//! headings, emphasis, lists, links, and code come out styled instead of littered with `#` and `*`.
//!
//! Two rules shape this module.
//!
//! **Color is semantic** (design-system §1): every style here resolves through a [`Role`] and the
//! active theme, never a literal color. That is what [`BeeStyleSheet`] is for — `tui-markdown` 0.3.8
//! exposes a `StyleSheet` seam precisely so a host can supply its own look, so bee supplies its own
//! rather than accepting the crate's hard-coded cyan-and-black. `NO_COLOR` therefore degrades this
//! path exactly like every other one, for free.
//!
//! **Every returned [`Line`] is exactly one row.** `view::chat_items` relies on that for exact scroll
//! arithmetic, so wrapping happens here rather than being left to `Paragraph`.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use tui_markdown::{Options, StyleSheet};

use super::theme_bridge::{dim_style, role_style};
use crate::viz::theme::Role;

/// bee's markdown look, expressed in semantic roles (design-system §1).
///
/// `Info` carries headings for the same reason it carries banners — a heading is a section's banner.
/// `Accent` carries both code and links, which the role table already names as its territory
/// ("highlights, steering, links"); the underline is what separates a link from a code span, and it
/// survives `NO_COLOR`, where the two would otherwise be indistinguishable.
#[derive(Debug, Clone, Copy, Default)]
pub struct BeeStyleSheet;

impl StyleSheet for BeeStyleSheet {
    fn heading(&self, level: u8) -> Style {
        match level {
            1 => role_style(Role::Info).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            2 => role_style(Role::Info).add_modifier(Modifier::BOLD),
            _ => role_style(Role::Info).add_modifier(Modifier::ITALIC),
        }
    }

    fn code(&self) -> Style {
        role_style(Role::Accent)
    }

    fn link(&self) -> Style {
        role_style(Role::Accent).add_modifier(Modifier::UNDERLINED)
    }

    fn blockquote(&self) -> Style {
        dim_style().add_modifier(Modifier::ITALIC)
    }

    fn heading_meta(&self) -> Style {
        dim_style()
    }

    fn metadata_block(&self) -> Style {
        dim_style()
    }
}

/// Render `md` as chat rows, each exactly one row tall at `width` columns.
pub fn render(md: &str, width: u16) -> Vec<Line<'static>> {
    let text = tui_markdown::from_str_with_options(md, &Options::new(BeeStyleSheet));
    let width = width.max(1);
    text.lines
        .into_iter()
        .flat_map(|line| wrap_line(line, width))
        .collect()
}

/// Break one styled line into rows of at most `width` columns, preserving each span's style.
///
/// A line that already fits is passed through **untouched** — which is what keeps a code block's
/// leading indentation intact, since re-flowing is the thing that would eat it. Only an overflowing
/// line is broken, greedily at the last space that fits, and only such a line can lose alignment.
fn wrap_line(line: Line<'_>, width: u16) -> Vec<Line<'static>> {
    let line_style = line.style;
    let owned = |spans: Vec<Span<'static>>| Line::from(spans).style(line_style);

    if line.width() <= width as usize {
        return vec![owned(
            line.spans
                .iter()
                .map(|s| Span::styled(s.content.to_string(), s.style))
                .collect(),
        )];
    }

    // Overflowing: fall to a char-level walk. Display width and char count diverge for wide glyphs;
    // the fits-check above uses ratatui's own width, so only an already-overflowing line can be
    // broken slightly early — never a correct line broken wrongly.
    let chars: Vec<(char, Style)> = line
        .spans
        .iter()
        .flat_map(|s| s.content.chars().map(move |c| (c, s.style)))
        .collect();

    let mut out = Vec::new();
    let mut start = 0usize;
    let width = width as usize;
    while start < chars.len() {
        if chars.len() - start <= width {
            out.push(owned(coalesce(&chars[start..])));
            break;
        }
        let hard = start + width;
        // Prefer the last space that fits; a single unbroken word longer than the pane is cut at the
        // margin rather than pushed off the edge.
        let brk = (start..hard)
            .rev()
            .find(|&i| chars[i].0.is_whitespace())
            .unwrap_or(hard);
        let brk = if brk == start { hard } else { brk };
        out.push(owned(coalesce(&chars[start..brk])));
        start = brk;
        while start < chars.len() && chars[start].0.is_whitespace() {
            start += 1;
        }
    }
    out
}

/// Rebuild spans from a `(char, Style)` run, merging neighbours that share a style so a wrapped line
/// carries a handful of spans rather than one per character.
fn coalesce(chars: &[(char, Style)]) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    for (c, style) in chars {
        match out.last_mut() {
            Some(last) if last.style == *style => last.content.to_mut().push(*c),
            _ => out.push(Span::styled(c.to_string(), *style)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The plain text of a rendered line, styles discarded.
    fn text_of(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn emphasis_becomes_style_rather_than_literal_punctuation() {
        let lines = render("plain **bold** tail\n", 40);
        let joined: String = lines.iter().map(text_of).collect();
        assert_eq!(joined, "plain bold tail", "the asterisks were styling");
        let bold: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(bold, "bold", "…and became weight instead");
    }

    #[test]
    fn a_heading_is_styled_marker_and_all() {
        // `tui-markdown` keeps the `# ` marker as content and styles the whole *line* rather than its
        // spans — worth pinning, since both facts drive what `wrap_line` has to preserve. The
        // modifiers are bee's (`BeeStyleSheet::heading`), not the crate's stock look; they are
        // asserted rather than the color because weight is what survives `NO_COLOR`.
        let lines = render("# Title\n", 40);
        assert_eq!(text_of(&lines[0]), "# Title");
        assert!(lines[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD | Modifier::UNDERLINED));
    }

    #[test]
    fn a_code_block_keeps_its_content_and_carries_one_style_throughout() {
        let lines = render("```\nlet x = 1;\n```\n", 40);
        let texts: Vec<String> = lines.iter().map(text_of).collect();
        assert!(
            texts.iter().any(|t| t == "let x = 1;"),
            "the code survived verbatim: {texts:?}"
        );
        // Every row of the block — fences included — carries the same line style, so the block reads
        // as one object.
        let styles: Vec<_> = lines.iter().map(|l| l.style).collect();
        assert!(
            styles.windows(2).all(|w| w[0] == w[1]),
            "one style across the block: {styles:?}"
        );
    }

    #[test]
    fn bold_runs_keep_their_style_when_a_long_line_is_wrapped() {
        let md = format!("start {} **emphatic** end", "filler ".repeat(12));
        let lines = render(&md, 20);
        assert!(lines.len() > 1, "the line overflowed and was broken");
        for line in &lines {
            assert!(
                line.width() <= 20,
                "every row fits the pane: {:?}",
                text_of(line)
            );
        }
        let bold_text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            bold_text.contains("emphatic"),
            "emphasis survived the wrap: {bold_text:?}"
        );
    }

    #[test]
    fn a_fitting_line_is_passed_through_untouched() {
        // Re-flowing is what would eat leading whitespace, so a line that fits is never re-flowed.
        // (An *indented* code block is normalized to a fenced one by the parser, which is why this
        // asserts on inline leading space rather than on the four-space form.)
        let lines = render("text with  a  gap\n", 40);
        assert_eq!(
            lines.iter().map(text_of).collect::<Vec<_>>(),
            vec!["text with  a  gap"],
            "interior spacing is preserved when nothing has to break"
        );
    }

    #[test]
    fn an_unbroken_word_longer_than_the_pane_is_cut_not_dropped() {
        let lines = render(&"x".repeat(50), 10);
        let joined: String = lines.iter().map(|l| text_of(l)).collect();
        assert_eq!(joined.matches('x').count(), 50, "no characters were lost");
        for line in &lines {
            assert!(line.width() <= 10);
        }
    }

    #[test]
    fn plain_prose_survives_untouched() {
        let lines = render("just a sentence.", 40);
        assert_eq!(
            lines.iter().map(text_of).collect::<Vec<_>>(),
            vec!["just a sentence."]
        );
    }
}
