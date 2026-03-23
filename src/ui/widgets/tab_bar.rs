use ratatui::{prelude::*, style::Modifier};

use crate::theme::Theme;

pub struct TabBarEntry<'a> {
    pub title: &'a str,
    pub format_name: Option<String>,
    pub num_visible: usize,
    pub tail_mode: bool,
    pub raw_mode: bool,
    pub paused: bool,
    pub retry_attempt: Option<u32>,
    pub has_lines: bool,
}

pub struct TabBar<'a> {
    pub tabs: Vec<TabBarEntry<'a>>,
    pub active_tab: usize,
    pub loading_info: Vec<(usize, usize)>,
    pub filtering_tabs: Vec<(usize, usize)>,
    pub show_borders: bool,
    pub mode_name: Option<&'a str>,
    pub theme: &'a Theme,
}

fn tab_suffix(
    entry: &TabBarEntry<'_>,
    is_active: bool,
    loading_info: &[(usize, usize)],
    filter_pct: Option<usize>,
    idx: usize,
) -> String {
    if let Some(&(_, pct)) = loading_info.iter().find(|(load_idx, _)| *load_idx == idx) {
        return format!(" {}% ", pct);
    }
    if let Some(pct) = filter_pct {
        if pct < 100 {
            return format!(" Filtering\u{2026} {}% ", pct);
        } else {
            return " Indexing\u{2026} ".to_string();
        }
    }
    if let Some(attempt) = entry.retry_attempt {
        return format!(" [RETRY #{}] ", attempt);
    }
    if is_active {
        let fmt_label = if entry.raw_mode {
            String::new()
        } else {
            match &entry.format_name {
                Some(name) => format!(" [{}]", name),
                None if entry.num_visible == 0 => String::new(),
                None => " [unknown format]".to_string(),
            }
        };
        format!(
            " ({}){}{}{}{}  ",
            entry.num_visible,
            if entry.tail_mode { " [TAIL]" } else { "" },
            if entry.raw_mode { " [RAW]" } else { "" },
            if entry.paused { " [PAUSED]" } else { "" },
            fmt_label,
        )
    } else if entry.raw_mode {
        " ".to_string()
    } else {
        match &entry.format_name {
            Some(name) => format!(" [{}] ", name),
            None if entry.has_lines => " [unknown format] ".to_string(),
            None => " ".to_string(),
        }
    }
}

impl<'a> Widget for TabBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = Style::default().fg(self.theme.border);
        let mode_style = Style::default()
            .fg(self.theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD);

        let mut spans: Vec<Span> = Vec::new();
        let mut used_width: usize = 0;

        if self.show_borders {
            spans.push(Span::styled("\u{250C}", border_style));
            used_width += 1;
        }

        if let Some(m) = self.mode_name {
            let text = format!(" [{}] ", m);
            used_width += unicode_width::UnicodeWidthStr::width(text.as_str());
            spans.push(Span::styled(text, mode_style));
        }

        for (i, entry) in self.tabs.iter().enumerate() {
            let is_active = i == self.active_tab;
            let tab_style = if is_active {
                Style::default()
                    .fg(self.theme.border_title)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(self.theme.inactive_tab_fg)
                    .bg(self.theme.root_bg)
            };
            let filter_pct = self
                .filtering_tabs
                .iter()
                .find(|(idx, _)| *idx == i)
                .map(|(_, p)| *p);
            let suffix = tab_suffix(entry, is_active, &self.loading_info, filter_pct, i);
            let tab_text = format!(" {}{}", entry.title, suffix);
            used_width += unicode_width::UnicodeWidthStr::width(tab_text.as_str());
            spans.push(Span::styled(tab_text, tab_style));

            if !self.show_borders {
                spans.push(Span::styled(" ", Style::default().bg(self.theme.root_bg)));
                used_width += 1;
            }
        }

        if self.show_borders {
            let total = area.width as usize;
            let fill = total.saturating_sub(used_width + 1);
            if fill > 0 {
                spans.push(Span::styled("\u{2500}".repeat(fill), border_style));
            }
            spans.push(Span::styled("\u{2510}", border_style));
        }

        Line::from(spans).render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;
    use ratatui::{Terminal, backend::TestBackend};

    fn make_entry(title: &str) -> TabBarEntry<'_> {
        TabBarEntry {
            title,
            format_name: None,
            num_visible: 10,
            tail_mode: false,
            raw_mode: false,
            paused: false,
            retry_attempt: None,
            has_lines: true,
        }
    }

    #[test]
    fn test_tab_bar_renders_single_tab() {
        let theme = Theme::default();
        let tabs = vec![make_entry("log.txt")];
        let tab_bar = TabBar {
            tabs,
            active_tab: 0,
            loading_info: vec![],
            filtering_tabs: vec![],
            show_borders: true,
            mode_name: None,
            theme: &theme,
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 1)).unwrap();
        terminal
            .draw(|f| f.render_widget(tab_bar, f.area()))
            .unwrap();
    }

    #[test]
    fn test_tab_bar_renders_multiple_tabs() {
        let theme = Theme::default();
        let tabs = vec![make_entry("a.log"), make_entry("b.log")];
        let tab_bar = TabBar {
            tabs,
            active_tab: 1,
            loading_info: vec![],
            filtering_tabs: vec![],
            show_borders: false,
            mode_name: Some("FILTER"),
            theme: &theme,
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 1)).unwrap();
        terminal
            .draw(|f| f.render_widget(tab_bar, f.area()))
            .unwrap();
    }

    #[test]
    fn test_tab_suffix_loading() {
        let entry = make_entry("t");
        let suffix = tab_suffix(&entry, true, &[(0, 42)], None, 0);
        assert_eq!(suffix, " 42% ");
    }

    #[test]
    fn test_tab_suffix_filtering_in_progress() {
        let entry = make_entry("t");
        let suffix = tab_suffix(&entry, false, &[], Some(55), 0);
        assert!(suffix.contains("55%"));
    }

    #[test]
    fn test_tab_suffix_indexing_complete() {
        let entry = make_entry("t");
        let suffix = tab_suffix(&entry, false, &[], Some(100), 0);
        assert!(suffix.contains("Indexing"));
    }

    #[test]
    fn test_tab_suffix_retry() {
        let mut entry = make_entry("t");
        entry.retry_attempt = Some(3);
        let suffix = tab_suffix(&entry, false, &[], None, 0);
        assert!(suffix.contains("RETRY #3"));
    }

    #[test]
    fn test_tab_suffix_active_with_format() {
        let entry = TabBarEntry {
            title: "t",
            format_name: Some("json".to_string()),
            num_visible: 5,
            tail_mode: true,
            raw_mode: false,
            paused: false,
            retry_attempt: None,
            has_lines: true,
        };
        let suffix = tab_suffix(&entry, true, &[], None, 0);
        assert!(suffix.contains("(5)"));
        assert!(suffix.contains("[TAIL]"));
        assert!(suffix.contains("[json]"));
    }
}
