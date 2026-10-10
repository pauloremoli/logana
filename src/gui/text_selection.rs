//! Stable text coordinates and GPUI geometry for a virtualized log document.
use std::{cell::RefCell, collections::HashMap, ops::Range, rc::Rc};

use crate::shared::{TabId, TabState};
use gpui_kit::base::{
    TextSelectionContentKey, TextSelectionEvent, TextSelectionHandle, TextSelectionRegistration,
    TextSelectionRun,
};
use gpui_kit::gpui::{
    self, AnyElement, App, Bounds, Element, ElementId, GlobalElementId, HitboxBehavior,
    InspectorElementId, IntoElement, LayoutId, Pixels, Point, StyledText, TextLayout, Window,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Endpoint {
    pub line: usize,
    pub row: usize,
    pub column: usize,
    pub offset: usize,
}
#[derive(Default)]
pub struct Document {
    pub tab: Option<TabId>,
    pub columns: Vec<String>,
    pub selection: Option<(Endpoint, Endpoint)>,
    pub dragged: bool,
    origin: Point<Pixels>,
    runs: Vec<Run>,
    keys: HashMap<u64, Endpoint>,
    next_key: u64,
}
struct Run {
    endpoint: Endpoint,
    text: gpui::SharedString,
    layout: TextLayout,
    bounds: Bounds<Pixels>,
    order: u64,
}
pub type SharedDocument = Rc<RefCell<Document>>;

impl Document {
    pub fn clear(&mut self) {
        self.selection = None;
        self.dragged = false;
        self.keys.clear();
    }
    fn hit(&mut self, point: Point<Pixels>) -> Option<TextSelectionContentKey> {
        let point = point + self.origin;
        let run = self.runs.iter().min_by(|a, b| {
            let distance = |r: &Run| {
                let dy = (f32::from(r.bounds.top()) - f32::from(point.y)).max(0.)
                    + (f32::from(point.y) - f32::from(r.bounds.bottom())).max(0.);
                let dx = (f32::from(r.bounds.left()) - f32::from(point.x)).max(0.)
                    + (f32::from(point.x) - f32::from(r.bounds.right())).max(0.);
                (dy, dx)
            };
            distance(a).partial_cmp(&distance(b)).unwrap()
        })?;
        let mut endpoint = run.endpoint.clone();
        endpoint.offset += run
            .layout
            .index_for_position(point)
            .unwrap_or_else(|i| i)
            .min(run.text.len());
        self.next_key += 1;
        self.keys.insert(self.next_key, endpoint);
        Some(TextSelectionContentKey::new(self.next_key))
    }
    pub fn sync(&mut self, snapshot: Option<gpui_kit::base::TextSelectionSnapshot>) {
        let Some(snapshot) = snapshot else {
            self.selection = None;
            return;
        };
        if let (Some(a), Some(b)) = (
            snapshot.anchor().content_key(),
            snapshot.cursor().content_key(),
        ) && let (Some(a), Some(b)) = (
            self.keys.get(&a.value()).cloned(),
            self.keys.get(&b.value()).cloned(),
        ) {
            self.dragged |= a != b;
            self.selection = Some((a, b));
        }
    }
    pub fn range(&self, row: usize, column: usize, length: usize) -> Option<Range<usize>> {
        let (a, b) = self.selection.as_ref()?;
        let a = (a.row, a.column, a.offset);
        let b = (b.row, b.column, b.offset);
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let cell = (row, column);
        if cell < (lo.0, lo.1) || cell > (hi.0, hi.1) {
            return None;
        }
        let start = if cell == (lo.0, lo.1) {
            lo.2.min(length)
        } else {
            0
        };
        let end = if cell == (hi.0, hi.1) {
            hi.2.min(length)
        } else {
            length
        };
        (start < end).then_some(start..end)
    }
    pub fn copy(&self, tab: &TabState) -> String {
        let Some((a, b)) = &self.selection else {
            return String::new();
        };
        let position = |e: &Endpoint| {
            (tab.filter.visible_indices.get_opt(e.row) == Some(e.line))
                .then_some((e.row, e.column, e.offset))
        };
        let (Some(a), Some(b)) = (position(a), position(b)) else {
            return String::new();
        };
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        if lo == hi {
            return String::new();
        }
        let mut rows = Vec::new();
        for row in lo.0..=hi.0 {
            let Some(line) = tab.filter.visible_indices.get_opt(row) else {
                continue;
            };
            let values = crate::gui::view::log_pane::displayed_values(tab, line, &self.columns);
            let first = if row == lo.0 { lo.1 } else { 0 };
            let last = if row == hi.0 {
                hi.1
            } else {
                values.len().saturating_sub(1)
            };
            let mut cells = Vec::new();
            for (column, text) in values.iter().enumerate().take(last + 1).skip(first) {
                let start = if (row, column) == (lo.0, lo.1) {
                    boundary(text, lo.2)
                } else {
                    0
                };
                let end = if (row, column) == (hi.0, hi.1) {
                    boundary(text, hi.2)
                } else {
                    text.len()
                };
                cells.push(text[start..end].to_string());
            }
            rows.push(cells.join("\t"));
        }
        rows.join("\n")
    }
}
fn boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

pub fn new_handle(
    document: SharedDocument,
    cx: &mut gpui::Context<crate::gui::app::App>,
) -> TextSelectionHandle {
    let handle = TextSelectionHandle::new("", cx);
    let hit_document = document.clone();
    handle.resolve_content_key_with(move |point, _| hit_document.borrow_mut().hit(point), cx);
    let cleared = document.clone();
    handle.clear_with(move |_| cleared.borrow_mut().clear(), cx);
    let weak = cx.entity().downgrade();
    let focus = weak.clone();
    handle.focus_with(
        move |window, cx| {
            if let Some(app) = focus.upgrade() {
                app.read(cx).root_focus.clone().focus(window, cx);
            }
        },
        cx,
    );
    let observer = weak.clone();
    handle
        .subscribe(
            move |event, cx| {
                if matches!(
                    event,
                    TextSelectionEvent::SelectionChanged(_) | TextSelectionEvent::Cleared
                ) && let Some(app) = observer.upgrade()
                {
                    cx.notify(app.entity_id());
                }
            },
            cx,
        )
        .detach();
    handle.copy_with(
        move |cx| {
            weak.upgrade()
                .map(|app| {
                    let app = app.read(cx);
                    document.borrow_mut().sync(app.text_handle.snapshot(cx));
                    let document = document.borrow();
                    app.state
                        .tabs
                        .iter()
                        .find(|tab| Some(tab.id) == document.tab)
                        .map(|tab| document.copy(tab))
                        .unwrap_or_default()
                })
                .unwrap_or_default()
        },
        cx,
    );
    handle
}

/// One participant covers the viewport; endpoints use stable source coordinates.
pub struct TableSelection {
    pub child: AnyElement,
    pub document: SharedDocument,
    pub handle: TextSelectionHandle,
    pub scroll: gpui_kit::gpui::ListState,
    pub row_height: Pixels,
}
impl IntoElement for TableSelection {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for TableSelection {
    type RequestLayoutState = ();
    type PrepaintState = gpui::Hitbox;
    fn id(&self) -> Option<ElementId> {
        Some("log-text-selection".into())
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> gpui::Hitbox {
        {
            let mut doc = self.document.borrow_mut();
            doc.origin = bounds.origin;
            doc.runs.clear();
        }
        let previous_width = self.scroll.viewport_bounds().size.width;
        self.child.prepaint(window, cx);
        if previous_width != self.scroll.viewport_bounds().size.width {
            // GPUI clears offscreen height hints when the list width changes.
            self.scroll
                .clone()
                .with_uniform_item_height(self.row_height);
        }
        self.document.borrow_mut().origin =
            bounds.origin + self.scroll.scroll_px_offset_for_scrollbar();
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let registration = TextSelectionRegistration::new(hitbox.clone(), bounds)
            .with_scroll_offset(self.scroll.scroll_px_offset_for_scrollbar())
            .with_text_bounds(
                self.document
                    .borrow()
                    .runs
                    .iter()
                    .map(|r| r.bounds)
                    .collect(),
            )
            .with_rendered_element(&self.handle, window, cx);
        self.handle.register(registration, window, cx);
        hitbox
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        hitbox: &mut gpui::Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        let scroll = self.scroll.clone();
        let hitbox = hitbox.id;
        let view = window.current_view();
        window.on_mouse_event(move |event: &gpui::ScrollWheelEvent, phase, window, cx| {
            if !phase.capture() || !hitbox.should_handle_scroll(window) {
                return;
            }
            let gpui::ScrollDelta::Lines(delta) = event.delta else {
                return;
            };
            if delta.y == 0. || delta.x.abs() > delta.y.abs() {
                return;
            }
            let mut offset = scroll.scroll_px_offset_for_scrollbar();
            offset.y = gpui::px(half_page_offset(
                f32::from(offset.y),
                f32::from(scroll.viewport_bounds().size.height),
                f32::from(
                    scroll.viewport_bounds().size.height + scroll.max_offset_for_scrollbar().y,
                ),
                delta.y,
            ));
            scroll.set_offset_from_scrollbar(offset);
            cx.notify(view);
            cx.stop_propagation();
        });
        self.child.paint(window, cx);
        let runs = self
            .document
            .borrow()
            .runs
            .iter()
            .map(|r| {
                TextSelectionRun::new(r.text.clone(), r.layout.clone(), r.bounds)
                    .with_document_order(r.order)
            })
            .collect::<Vec<_>>();
        self.handle.update_runs(&runs, cx);
    }
}

/// Mouse wheel notches move by half the viewport, clamped to the document.
fn half_page_offset(current: f32, viewport: f32, content: f32, wheel: f32) -> f32 {
    let bottom = (viewport - content).min(0.);
    (current + wheel.signum() * viewport * 0.5).clamp(bottom, 0.)
}

/// Styled glyph geometry also drives Unicode-safe hit testing and highlighting.
pub struct SelectionText {
    pub text: gpui::SharedString,
    pub styled: StyledText,
    pub document: SharedDocument,
    pub endpoint: Endpoint,
    pub order: u64,
}
impl IntoElement for SelectionText {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for SelectionText {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.styled.request_layout(id, inspector, window, cx)
    }
    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.styled
            .prepaint(id, inspector, bounds, &mut (), window, cx);
        self.document.borrow_mut().runs.push(Run {
            endpoint: self.endpoint.clone(),
            text: self.text.clone(),
            layout: self.styled.layout().clone(),
            bounds,
            order: self.order,
        });
    }
    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.styled
            .paint(id, inspector, bounds, &mut (), &mut (), window, cx);
    }
}

#[cfg(all(test, feature = "gui-test"))]
impl Document {
    pub(crate) fn test_point(&self, line: usize, column: usize, offset: usize) -> Point<Pixels> {
        self.runs
            .iter()
            .find_map(|run| {
                if run.endpoint.line != line || run.endpoint.column != column {
                    return None;
                }
                let start = run.endpoint.offset;
                if offset < start || offset > start + run.text.len() {
                    return None;
                }
                run.layout
                    .position_for_index(offset - start)
                    .map(|p| p + gpui::point(gpui::px(0.5), gpui::px(8.)))
            })
            .expect("text coordinate must be mounted")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn mouse_wheel_moves_half_a_page_and_clamps_at_document_edges() {
        assert_eq!(super::half_page_offset(0., 600., 2000., -3.), -300.);
        assert_eq!(super::half_page_offset(-600., 600., 2000., 3.), -300.);
        assert_eq!(super::half_page_offset(-1300., 600., 2000., -1.), -1400.);
        assert_eq!(super::half_page_offset(-100., 600., 2000., 1.), 0.);
        assert_eq!(super::half_page_offset(0., 600., 200., -1.), 0.);
        assert_eq!(super::half_page_offset(0., 300., 2000., -1.), -150.);
    }

    use super::*;
    use crate::{
        db::{Database, LogManager},
        ingestion::FileReader,
    };
    use std::sync::Arc;

    async fn tab(bytes: &[u8]) -> TabState {
        let db = Arc::new(Database::in_memory().await.unwrap());
        TabState::new(
            FileReader::from_bytes(bytes.to_vec()),
            LogManager::new(db, None).await,
            "test.log".into(),
        )
    }
    fn endpoint(row: usize, column: usize, offset: usize) -> Endpoint {
        Endpoint {
            line: row,
            row,
            column,
            offset,
        }
    }
    #[tokio::test]
    async fn forward_reverse_unicode_and_unmounted_rows_copy_identically() {
        let tab = tab("hello 東京\nsecond\nthird\n".as_bytes()).await;
        let mut doc = Document {
            columns: vec!["message".into()],
            selection: Some((endpoint(0, 0, 6), endpoint(2, 0, 3))),
            ..Default::default()
        };
        assert!(doc.runs.is_empty()); // Copy must not depend on mounted rows.
        assert_eq!(doc.copy(&tab), "東京\nsecond\nthi");
        let (a, b) = doc.selection.take().unwrap();
        doc.selection = Some((b, a));
        assert_eq!(doc.copy(&tab), "東京\nsecond\nthi");
        assert_eq!(doc.range(1, 0, 6), Some(0..6));
        assert_eq!(doc.range(0, 0, 12), Some(6..12));
    }
    #[tokio::test]
    async fn column_order_hidden_fields_and_wrapping_do_not_change_copy_text() {
        let mut tab = tab(b"{\"message\":\"hello world\",\"tenant\":\"pay\",\"secret\":\"hidden\"}\n{\"message\":\"next\",\"tenant\":\"api\",\"secret\":\"hidden\"}\n").await;
        tab.display.format = Some(Arc::from(
            crate::parser::find_builtin_parser("json").unwrap(),
        ));
        tab.invalidate_parse_cache();
        let fields = tab.collect_field_names();
        assert!(
            fields.contains(&"tenant".into()),
            "JSON schema should be detected"
        );
        tab.display.hidden_fields.insert("secret".into());
        tab.display.field_layout.columns = Some(vec!["tenant".into(), "message".into()]);
        let mut doc = Document {
            columns: vec!["tenant".into(), "message".into()],
            selection: Some((endpoint(0, 0, 1), endpoint(1, 1, 2))),
            ..Default::default()
        };
        assert_eq!(doc.copy(&tab), "ay\thello world\napi\tne");
        tab.display.wrap = true;
        assert_eq!(doc.copy(&tab), "ay\thello world\napi\tne");
        let (a, b) = doc.selection.take().unwrap();
        doc.selection = Some((b, a));
        assert_eq!(doc.copy(&tab), "ay\thello world\napi\tne");
        let raw = crate::gui::view::log_pane::raw_log_text(&String::from_utf8_lossy(
            &tab.file_reader.get_line(0),
        ));
        assert_eq!(
            raw,
            "{\"message\":\"hello world\",\"tenant\":\"pay\",\"secret\":\"hidden\"}"
        );
    }
    #[tokio::test]
    async fn stale_source_coordinates_and_empty_selection_copy_nothing() {
        let tab = tab(b"first\nsecond\n").await;
        let mut doc = Document {
            columns: vec!["message".into()],
            selection: Some((endpoint(0, 0, 1), endpoint(0, 0, 1))),
            ..Default::default()
        };
        assert!(doc.copy(&tab).is_empty());
        doc.selection.as_mut().unwrap().0.line = 42;
        assert!(doc.copy(&tab).is_empty());
        doc.clear();
        assert!(doc.copy(&tab).is_empty());
    }
    #[tokio::test]
    async fn raw_reader_copy_handles_crlf_and_a_literal_final_carriage_return() {
        let tab = tab(b"  first  \r\n\x1b[31msecond\x1b[0m\r\nfinal\r").await;
        use crate::gui::view::log_pane::raw_log_message;
        assert_eq!(raw_log_message(&tab.file_reader, 0), "  first  ");
        assert_eq!(
            raw_log_message(&tab.file_reader, 1),
            "\x1b[31msecond\x1b[0m"
        );
        assert_eq!(raw_log_message(&tab.file_reader, 2), "final\r");
    }
    #[tokio::test]
    async fn raw_copy_preserves_normalized_source_in_both_file_loading_paths() {
        use std::{io::Write, sync::atomic::AtomicBool};
        let original = b"\x1b[31m{\"message\":\"hello\",\"secret\":\"hidden\"}\x1b[0m\r\nsecond\r";
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(original).unwrap();
        let path = file.path().to_str().unwrap();
        let direct = FileReader::new(path).unwrap();
        let loaded = FileReader::load(
            path.into(),
            None,
            false,
            Arc::new(AtomicBool::new(false)),
            false,
        )
        .await
        .unwrap();
        let chunked = loaded.result_rx.await.unwrap().unwrap().reader;
        for mut reader in [direct, chunked] {
            assert_eq!(
                reader.get_raw_line(0).as_ref(),
                &original[..original.len() - 9]
            );
            assert_eq!(reader.get_raw_line(1).as_ref(), b"second\r");
            assert!(!reader.get_line(0).contains(&b'\x1b'));
            reader.append_bytes(b"\nnew\x1b[0m\n");
            assert_eq!(reader.get_raw_line(2).as_ref(), b"new\x1b[0m");
        }
    }
    #[test]
    fn raw_copy_only_removes_the_terminating_delimiter() {
        use crate::gui::view::log_pane::raw_log_text;
        assert_eq!(raw_log_text("  a\tb  \r\n"), "  a\tb  ");
        assert_eq!(raw_log_text("a\ninside\n"), "a\ninside");
        assert_eq!(raw_log_text("a\r"), "a\r");
        assert_eq!(boundary("東京", 2), 0);
    }
}
