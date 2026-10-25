use super::{TabState, parse_line_with_continuation};
use crate::filters::{self, FilterDecision, FilterDef};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::oneshot;

pub struct FilterNavigation {
    receiver: oneshot::Receiver<Option<usize>>,
    cancel: Arc<AtomicBool>,
    filter: FilterDef,
    generation: u64,
    from: usize,
}

enum Matcher {
    Text(Box<dyn filters::Filter>),
    Field(filters::FieldFilter),
    Date(filters::DateFilter),
}

impl Drop for FilterNavigation {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Matcher {
    fn new(def: &FilterDef) -> Option<Self> {
        if let Some(expr) = def.pattern.strip_prefix(filters::DATE_PREFIX) {
            return filters::parse_date_filter(expr).ok().map(Self::Date);
        }
        if let Some(expr) = def.pattern.strip_prefix(filters::FIELD_PREFIX) {
            let (conditions, text) = filters::parse_field_filter_expr(expr).ok()?;
            return Some(Self::Field(filters::FieldFilter {
                conditions,
                text,
                decision: FilterDecision::Include,
            }));
        }
        filters::build_filter(
            &def.pattern,
            FilterDecision::Include,
            true,
            0,
            def.use_regex,
            def.ignore_case,
        )
        .map(Self::Text)
    }
}

impl TabState {
    /// Navigate by a filter's predicate without changing the ordinary text search.
    /// Match the selected rule even when disabled; only visit currently visible rows.
    pub fn begin_selected_filter_match(&mut self, index: usize, forward: bool, count: usize) {
        self.filter.navigation = None;
        let Some(filter) = self.log_manager.get_filters().get(index).cloned() else {
            return;
        };
        let reader = self.file_reader.clone();
        let visible = self.filter.visible_indices.clone();
        let from = self.scroll.scroll_offset;
        let parser = self.display.format.clone();
        let source_parsers = self.merged.as_ref().map(|m| m.source_parsers.clone());
        let cmap = self.active_continuation_map().cloned();
        let years = self.year_map.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, receiver) = oneshot::channel();
        let definition = filter.clone();
        tokio::task::spawn_blocking(move || {
            let Some(matcher) = Matcher::new(&definition) else {
                return;
            };
            let len = visible.len();
            let mut matches = Vec::new();
            let count = count.max(1);
            for step in 1..=len {
                if worker_cancel.load(Ordering::Relaxed) {
                    return;
                }
                let start = from.min(len.saturating_sub(1));
                let row = if forward {
                    (start + step) % len
                } else {
                    (start + len - step % len) % len
                };
                let line = visible.get(row);
                let bytes = reader.get_line(line);
                let parser = if let Some(parsers) = &source_parsers {
                    reader
                        .merged_entries()
                        .and_then(|entries| entries.get(line))
                        .and_then(|entry| parsers.get(entry.source_idx))
                        .and_then(|p| p.as_deref())
                } else {
                    parser.as_deref()
                };
                let matched = match &matcher {
                    Matcher::Text(filter) => filter.matches(&bytes) == FilterDecision::Include,
                    Matcher::Date(filter) => parser
                        .and_then(|p| p.parse_timestamp(&bytes))
                        .is_some_and(|ts| {
                            filter.matches(ts, years.as_ref().map(|y| y.year_for_line(line)))
                        }),
                    Matcher::Field(filter) => parser
                        .and_then(|p| {
                            parse_line_with_continuation(
                                p,
                                &reader,
                                &bytes,
                                cmap.as_deref().map(Vec::as_slice),
                                line,
                            )
                        })
                        .is_some_and(|parts| filters::field_filter_matches(filter, &parts, &bytes)),
                };
                if matched {
                    matches.push(line);
                    if matches.len() == count {
                        break;
                    }
                }
            }
            let target = (!matches.is_empty()).then(|| matches[(count - 1) % matches.len()]);
            let _ = sender.send(target);
        });
        self.filter.navigation = Some(FilterNavigation {
            receiver,
            cancel,
            filter,
            generation: self.cache.parse_gen,
            from,
        });
        self.stream.tail_mode = false;
    }

    /// Nonblocking completion, shared by both frontend event loops.
    /// `Some(true)` means moved; `Some(false)` means no visible match.
    pub fn advance_selected_filter_match(&mut self) -> Option<bool> {
        let navigation = self.filter.navigation.as_ref()?;
        if navigation.generation != self.cache.parse_gen
            || navigation.from != self.scroll.scroll_offset
        {
            self.filter.navigation = None;
            return None;
        }
        let navigation = self.filter.navigation.as_mut().unwrap();
        let result = match navigation.receiver.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => return None,
            Err(oneshot::error::TryRecvError::Closed) => {
                self.filter.navigation = None;
                return None;
            }
        };
        let navigation = self.filter.navigation.take().unwrap();
        if !self.log_manager.get_filters().contains(&navigation.filter) {
            return None;
        }
        if let Some(line) = result {
            self.scroll_to_line_idx(line);
            self.stream.tail_mode = false;
            Some(true)
        } else {
            self.set_notification("No visible messages match the selected filter.");
            Some(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::{Database, LogManager},
        filters::{FilterOptions, FilterType},
        ingestion::FileReader,
    };

    async fn tab(text: &str, patterns: &[(&str, FilterOptions)]) -> TabState {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let mut manager = LogManager::new(db, None).await;
        for (pattern, options) in patterns {
            manager
                .add_filter_with_color((*pattern).into(), FilterType::Highlight, options.clone())
                .await;
        }
        TabState::new(
            FileReader::from_bytes(text.as_bytes().to_vec()),
            manager,
            "test".into(),
        )
    }

    async fn navigate(tab: &mut TabState, index: usize, forward: bool, count: usize) -> bool {
        tab.begin_selected_filter_match(index, forward, count);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(found) = tab.advance_selected_filter_match() {
                    return found;
                }
                assert!(
                    tab.filter.navigation.is_some(),
                    "navigation was unexpectedly discarded"
                );
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn selected_filter_navigation_wraps_counts_and_preserves_search() {
        let mut tab = tab(
            "hit a\nother\nhit b\nhit c\n",
            &[("hit", FilterOptions::default())],
        )
        .await;
        tab.search
            .query
            .search("other", 0..4, |i| {
                Some(String::from_utf8_lossy(&tab.file_reader.get_line(i)).into_owned())
            })
            .unwrap();
        tab.stream.tail_mode = true;
        for (forward, count, expected) in [
            (true, 1, 2),
            (true, 1, 3),
            (true, 1, 0),
            (false, 1, 3),
            (true, 5, 2),
        ] {
            assert!(navigate(&mut tab, 0, forward, count).await);
            assert_eq!(tab.scroll.scroll_offset, expected);
        }
        assert!(!tab.stream.tail_mode);
        assert_eq!(tab.search.query.get_pattern(), Some("other"));
        assert_eq!(tab.search.query.get_total_match_count(), 1);
    }

    #[tokio::test]
    async fn selected_filter_navigation_uses_literal_regex_case_and_visible_rows() {
        let mut tab = tab(
            "start\nHIT one\nhit.two\nHIT three\n",
            &[
                ("hit.", FilterOptions::default()),
                ("^hit", FilterOptions::default().regex().ignore_case()),
            ],
        )
        .await;
        assert!(navigate(&mut tab, 0, true, 1).await);
        assert_eq!(tab.scroll.scroll_offset, 2);
        tab.filter.visible_indices = super::super::VisibleLines::Filtered(vec![0, 1, 3]);
        tab.scroll.scroll_offset = 0;
        assert!(navigate(&mut tab, 1, true, 1).await);
        assert_eq!(tab.scroll.scroll_offset, 1);
        assert!(navigate(&mut tab, 1, true, 1).await);
        assert_eq!(tab.scroll.scroll_offset, 2);
        assert!(!navigate(&mut tab, 0, true, 1).await);
        assert_eq!(tab.scroll.scroll_offset, 2);
    }

    #[tokio::test]
    async fn selected_filter_navigation_matches_compound_fields_and_dates_in_raw_mode() {
        let field =
            filters::encode_field_filter(&[("level".into(), "ERROR".into())], Some("timeout"));
        let mut tab = tab(concat!(
            "{\"timestamp\":\"2024-01-01T10:00:00Z\",\"level\":\"INFO\",\"message\":\"start\"}\n",
            "{\"timestamp\":\"2024-01-01T11:00:00Z\",\"level\":\"ERROR\",\"message\":\"timeout\"}\n",
            "{\"timestamp\":\"2024-01-01T12:00:00Z\",\"level\":\"ERROR\",\"message\":\"other\"}\n"),
            &[(&field, FilterOptions::default()),("@date:>= 12:00:00", FilterOptions::default())]).await;
        tab.display.raw_mode = true;
        tab.filter.enabled = false;
        tab.refresh_visible();
        assert!(navigate(&mut tab, 0, true, 1).await);
        assert_eq!(tab.scroll.scroll_offset, 1);
        assert!(navigate(&mut tab, 1, true, 1).await);
        assert_eq!(tab.scroll.scroll_offset, 2);
    }

    #[tokio::test]
    async fn selected_filter_navigation_ignores_rule_enablement_and_cancels_stale_results() {
        let mut tab = tab("start\nhit\n", &[("hit", FilterOptions::default())]).await;
        let id = tab.log_manager.get_filters()[0].id;
        tab.log_manager.toggle_filter(id).await;
        assert!(navigate(&mut tab, 0, true, 1).await);
        assert_eq!(tab.scroll.scroll_offset, 1);
        tab.scroll.scroll_offset = 0;
        tab.begin_selected_filter_match(0, true, 1);
        let cancel = tab.filter.navigation.as_ref().unwrap().cancel.clone();
        tab.filter.navigation = None;
        assert!(cancel.load(Ordering::Relaxed));
        tab.begin_selected_filter_match(0, true, 1);
        tab.invalidate_parse_cache();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while tab.filter.navigation.is_some() {
                assert_eq!(tab.advance_selected_filter_match(), None);
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(tab.scroll.scroll_offset, 0);
    }

    #[tokio::test]
    async fn selected_filter_navigation_handles_empty_logs() {
        let mut tab = tab("", &[("hit", FilterOptions::default())]).await;
        assert!(!navigate(&mut tab, 0, false, 1).await);
    }
}
