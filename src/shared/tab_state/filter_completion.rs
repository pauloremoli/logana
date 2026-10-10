use super::{FilterHandle, TabState, VisibleLines};
use crate::filters::FilterManager;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{mpsc, oneshot};

pub struct CompletedView {
    visible: VisibleLines,
    baseline: Option<VisibleLines>,
    counts: Vec<usize>,
    marked: Vec<usize>,
    anchor: Option<usize>,
}

pub(super) struct Finalizer {
    continuation: Option<Arc<Vec<usize>>>,
    collapse: bool,
    overrides: HashSet<usize>,
    marked: Vec<usize>,
    anchor: Option<usize>,
}

impl Finalizer {
    pub(super) fn capture(tab: &TabState) -> Self {
        let mut marked = tab.mark_manager.get_indices();
        marked.retain(|&line| line < tab.file_reader.line_count());
        Self {
            continuation: tab.active_continuation_map().cloned(),
            collapse: tab.display.collapse_continuations,
            overrides: tab.filter.overridden_groups.clone(),
            marked,
            anchor: tab.filter.visible_indices.get_opt(tab.scroll.scroll_offset),
        }
    }

    pub(super) fn finish(
        self,
        mut visible: VisibleLines,
        counts: Vec<usize>,
        include: bool,
        correct: bool,
    ) -> CompletedView {
        if correct && let Some(map) = &self.continuation {
            super::apply_continuation_correction(&mut visible, map, include);
        }
        visible.union_marked(&self.marked);
        let baseline = (self.collapse || !self.overrides.is_empty()).then(|| visible.clone());
        if baseline.is_some()
            && let Some(map) = &self.continuation
        {
            super::apply_collapse_correction(&mut visible, map, self.collapse, &self.overrides);
        }
        CompletedView {
            visible,
            baseline,
            counts,
            marked: self.marked,
            anchor: self.anchor,
        }
    }
}

impl TabState {
    pub(super) fn install_filter_completion(&mut self) -> bool {
        let Some(mut receiver) = self.filter.completed.take() else {
            return false;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => {
                self.filter.completed = Some(receiver);
                return false;
            }
            Err(oneshot::error::TryRecvError::Closed) => {
                self.filter.handle = None;
                return true;
            }
        };
        let mut marked = self.mark_manager.get_indices();
        marked.retain(|&line| line < self.file_reader.line_count());
        if marked != result.marked {
            self.begin_filter_refresh_background();
            return true;
        }
        self.filter.visible_indices = result.visible;
        self.filter.pre_collapse_visible = result.baseline;
        self.filter.match_counts = result.counts;
        self.filter.cached_scan = None;
        self.filter.handle = None;
        self.restore_scroll_to_line(result.anchor);
        true
    }

    pub(super) fn begin_simple_filter_refresh(&mut self) {
        let reader = self.file_reader.clone();
        let structured = self.display.format.is_some() && !self.display.raw_mode;
        let marks_only = self.filter.show_marks_only;
        let disabled = !self.filter.enabled;
        let manager = self.log_manager.clone();
        let finalizer = Finalizer::capture(self);
        let anchor = finalizer.anchor;
        let cancel = Arc::new(AtomicBool::new(false));
        let canceled = cancel.clone();
        let (result_tx, result_rx) = mpsc::channel(1);
        let (prepared_tx, prepared_rx) = oneshot::channel();
        let (completed_tx, completed_rx) = oneshot::channel();
        self.filter.saved_view = None;
        self.filter.prepared = Some(prepared_rx);
        self.filter.completed = Some(completed_rx);
        self.filter.handle = Some(FilterHandle {
            result_rx,
            cancel,
            displayed_progress: 0.,
            scroll_anchor: anchor,
            received_first_chunk: false,
            scan_fingerprint: Vec::new(),
            scan_line_count: reader.line_count(),
            scan_raw_mode: self.display.raw_mode,
            scan_highlight_mode: self.filter.highlight_mode,
            scan_group_fingerprint: Vec::new(),
        });
        tokio::task::spawn_blocking(move || {
            let _result_tx = result_tx;
            if canceled.load(Ordering::Relaxed) {
                return;
            }
            let prepared = if marks_only {
                let (fm, text, date, field) = manager.build_filter_manager();
                (Arc::new(fm), text, date, field)
            } else {
                (
                    Arc::new(FilterManager::empty()),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                )
            };
            if prepared_tx.send(prepared).is_err() {
                return;
            }
            let visible = if marks_only {
                VisibleLines::Filtered(finalizer.marked.clone())
            } else if disabled {
                VisibleLines::All(reader.line_count())
            } else {
                Self::unfiltered_visible_for(&reader, structured, &canceled)
            };
            if canceled.load(Ordering::Relaxed) {
                return;
            }
            let result = finalizer.finish(visible, Vec::new(), false, false);
            let _ = completed_tx.send(result);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalization_groups_rows_merges_marks_and_retains_collapse_baseline() {
        let finalizer = Finalizer {
            continuation: Some(Arc::new(vec![0, 0, 2, 2])),
            collapse: true,
            overrides: HashSet::new(),
            marked: vec![2],
            anchor: Some(2),
        };
        let result = finalizer.finish(VisibleLines::Filtered(vec![0]), vec![1], true, true);
        assert_eq!(result.visible.iter().collect::<Vec<_>>(), vec![0, 2]);
        assert_eq!(
            result.baseline.unwrap().iter().collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(result.counts, vec![1]);
        assert_eq!(result.anchor, Some(2));
    }
}
