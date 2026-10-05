# GUI Parity Backlog

Reviewed against the native GPUI frontend on 2026-10-05. Checked items describe
working GUI behavior, not merely shared TUI code. Features remain additive;
GUI-only builds must not acquire terminal dependencies.

## Implemented

- [x] Open files using a native dialog and background preview/full loading.
- [x] Switch and close tabs with the mouse.
- [x] Native Actions menu lists available actions, without the last executed command.
- [x] Native filter and group editors: conditions, styles, membership, toggles,
      ordering, deletion, and database persistence.
- [x] Search, next/previous matches, time-range filters, and field-value facets.
- [x] Choose, hide, and reorder displayed columns; resize and fit column widths.
- [x] Choose schemas and inspect/copy the source path.
- [x] Theme preview, cancel, apply, and persisted theme choice.
- [x] Value/level color controls and theme-aware rendering.
- [x] Display controls: wrapping, absolute/relative line numbers, keys, raw view,
      collapse/expand, sidebar visibility and position (currently tab-local).
- [x] Pause/resume/stop file watching and toggle follow mode. Incoming snapshots
      are consumed in background tasks, including paused-stream catch-up.
- [x] Mouse row selection and Shift-click ranges; cursor and row backgrounds.
- [x] Mouse text selection across columns/rows, forward/reverse, wrapped text,
      Unicode, edge scrolling, and virtualized rows outside the viewport.
- [x] Ctrl+C / Cmd+C and context-menu copying of selected displayed text.
- [x] Copy displayed row in visible column order; copy the clicked raw log entry
      including hidden fields and original syntax.
- [x] Full-width bookmarked row backgrounds, including wrapped continuations;
      row/cursor selection takes precedence and text highlights paint above it.
- [x] Notifications and focus handling.
- [x] Separate default TUI, GUI-only, combined, and core/schema feature builds.

## Missing parity features

- [x] Bookmark management page: list, jump, toggle/remove, and clear bookmarks.
- [x] Annotation editor and page: attach notes to source rows, edit/delete, jump.
- [x] Save current logs and marked logs using native save dialogs.
- [ ] Re-home saved temporary/stream tabs and migrate their context to the permanent path.
- [x] Save/load/append/import filter files using native dialogs.
- [x] Template exports with editable footer fields and output dialogs.
- [x] Consume live file growth, preserve filters and source indices, follow the
      newest visible row, and handle truncation/rotation and pause/resume.
- [ ] Persist and restore display settings, per-file context, and sessions with
      the existing restore policies.
- [ ] Keyboard navigation, tab shortcuts, bookmark shortcuts, file switching,
      and discoverable shortcut help. Legacy `KeyPressed` is currently ignored.
- [ ] Default-filter configuration and automatic schema-default loading.
- [ ] Merge-file picker and merged-source lifecycle.
- [ ] Archive-entry picker and extraction workflow.
- [ ] Docker container selection and log streaming.
- [ ] DLT source selection and streaming.
- [ ] OTEL receiver configuration and streaming.
- [x] Run subprocess output in a log tab with stop/pause lifecycle.
- [ ] Enable/disable MCP with the existing server lifecycle.
- [ ] Reset settings/database with explicit scope and restore-policy handling.

## Additional GUI pages

- [ ] Searches: define behavior for saved searches; the TUI has no equivalent
      saved-search page, so this is a GUI enhancement rather than parity.
- [ ] Settings: expose persisted application options in a native page.

## Validation

Mouse selection and raw copying have focused Unicode, wrapping, hidden-column,
virtualization, and original-byte tests plus native GPUI interaction tests.
Bookmark background painting has light/dark and selection-precedence tests.
Default TUI, GUI-only, combined, and no-feature library/schema checks pass.
Strict GUI Clippy currently reports unrelated existing editor/update lints.

Update the relevant checkbox and validation evidence in each feature commit.
