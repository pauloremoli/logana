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
      collapse/expand, sidebar visibility and position, with persisted defaults.
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
- [x] Persist and restore display defaults, per-file context, and ordered sessions.
      Shared database keys and independent Ask/Always/Never restore policies are
      honored, with config overrides, native restore prompts, and save flushing
      before GUI quit/window close. Closing the last tab clears the saved session;
      declining restoration in an empty window preserves the previous session.
      Temporary/stream sources are excluded from session reopening.
- [x] Keyboard navigation, tab shortcuts, bookmark shortcuts, file switching,
      and discoverable shortcut help. Shared bindings drive native GUI navigation.
- [x] Default-filter configuration and automatic schema-default loading.
- [ ] Merge-file picker and merged-source lifecycle.
- [ ] Archive-entry picker and extraction workflow.
- [x] Docker container selection and log streaming.
- [x] DLT source selection and streaming.
- [x] OTEL receiver configuration and streaming.
- [x] Run subprocess output in a log tab with stop/pause lifecycle.
- [x] Enable/disable MCP with the existing server lifecycle.
- [ ] Reset settings/database with explicit scope and restore-policy handling.


Update the relevant checkbox and validation evidence in each feature commit.

Persistence has database round-trip and native prompt tests covering policy
independence/config precedence, filtered cursor restoration, column layout,
bookmarks/annotations, ordered sessions, temporary sources, coalesced saves,
display edits, closing the last tab, and native viewport restoration after
asynchronous filtering.
