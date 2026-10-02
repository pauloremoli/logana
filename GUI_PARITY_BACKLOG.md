# GUI Parity Backlog

Tracks what's still missing in the GPUI frontend (`src/gui`) relative to the
TUI (`src/mode`, `src/ui`). Check items off as they land; add new ones as
gaps are found. Last reviewed: 2026-10-02 (gpui-rewrite branch).

## Commands not wired up

`execute_command` (`src/gui/update.rs`) implements `Filter`, `Exclude`,
`Highlight`, `ClearFilters`, `DisableFilters`, `EnableFilters`,
`ToggleGroup`, `Group`, `DateFilter`. `Pause`/`Resume`/`Stop`
(`apply_stream_command`) and `Wrap`/`LineNumbers`/`RelativeLineNumbers`/
`ShowKeys`/`HideKeys`/`Raw`/`Collapse`/`Expand` (`apply_display_command`)
are intercepted in `handle_command_string` before `execute_command`, since
neither touches `LogManager`. Everything else in `Commands` still errors
with "not yet supported in the GUI":

- [ ] Persistence: `Save`, `SaveFilters`, `LoadFilters`, `ImportFilters`,
      `Export`, `ExportMarked`
- [ ] Field layout: `HideField`, `ShowField`, `ShowAllFields`,
      `SelectFields` — unlike the rest of this list, `SelectFieldsMode`
      itself needs no new GUI plumbing (it mutates `tab.display.
      field_layout`/`hidden_fields` directly in `Mode::handle_key`, same
      as `FilterManagementMode`), but nothing in the GUI's log table
      reads either field yet (`log_pane.rs`'s Message cell always shows
      `row_message_bytes`, never a structured/field-aware rendering), so
      wiring the picker now would be a fully inert popup — skipped until
      there's a reason to read those fields
- [ ] Sources: `Merge`, `Docker`, `Dlt`, `Otel` (`Schema` is wired, see
      below)
- [ ] `Reset` (`Pause`/`Resume`/`Stop`/`Tail` already work, see below) —
      deliberately skipped, not just unstarted: it wipes the whole SQLite
      database (`db.reset_all()`) and touches app-wide fields `GuiState`
      doesn't have (`session.restore_policy`/`restore_file_policy`),
      unlike every other command on this list so far
- [ ] Misc: `EnableMcp`/`DisableMcp`, `DefaultFilters`, `Run`, `Path`
      (`SidebarPosition` is wired, see below)

`Wrap`/`LineNumbers`/`RelativeLineNumbers`/`Collapse`/`Expand` are a
deliberate simplification vs. the TUI: app-wide settings there (broadcast
to every open tab, persisted to `AppSettingsStore`), active-tab-only and
not persisted in the GUI, since `GuiState` doesn't load persisted display
settings on startup either. `ShowKeys`/`HideKeys`/`Raw` already matched
this shape in the TUI itself.

## Modes with no GUI rendering

`view::mod.rs` only renders `tab_bar`, `log_pane`, `sidebar`
(filter/group panes), `command_bar`, `search_bar`, `mode_bar`. These TUI
modes have working `Mode::handle_key` logic (shared automatically through
the `Box<dyn Mode>` boundary) but nothing draws their popup yet:

- [ ] `archive_picker_mode`
- [ ] `comment_mode`
- [ ] `default_filters_mode`
- [ ] `dlt_select_mode`
- [ ] `docker_select_mode`
- [ ] `export_footer_mode`
- [ ] `merge_select_mode`
- [ ] `select_fields_mode`
- [ ] `visual_char_mode` — only visual **line** selection is highlighted in
      `log_pane.rs`; char-wise visual selection has no rendering.
      Architectural snag, not just unstarted: `anchor_col`/`cursor_col` are
      char indices into `visual_char_mode::display_line_text(tab)`, the
      TUI's own single-line reconstruction (timestamp+level+message
      combined per `field_layout`) — not the same text as any one GUI
      column, so the `(lo, hi)` range can't be sliced directly onto the
      Message cell's text the way `row_message_bytes` works. Needs either
      a coordinate re-derivation onto the Message column's own text, or
      rendering the selected row's Message cell from
      `display_line_text(tab)` instead of `row_message_bytes` while the
      selection is active on it.

## Effect plumbing

`src/gui/effect.rs`'s `Effect` enum only covers `Quit`, `OpenFileDialog`,
`LoadFile`, `ToggleFilter`, `ToggleGroup`, `RemoveGroup`, `ExecuteCommand`.
Even once the modes above are rendered, there's no effect yet for:

- [ ] Save-file dialogs (filters, marked lines, export)
- [ ] Source-picker dialogs (merge file picker, docker container picker,
      archive entry picker)

## Already working (don't re-litigate)

- File open: dialog + background two-phase preview/full load, shared with
  `App::open_file`
- Tab switch/close (mouse + keyboard)
- Filter/group toggle (mouse + keyboard)
- Command palette overlay (`:` commands), search bar (click to enter
  `SearchMode`), time-range dropdown, `?` keybindings-help overlay,
  `:theme` picker (live preview, apply on Enter, revert on Esc — not
  persisted, see above), `:value-colors`/`:level-colors` pickers,
  `:set-theme <name>`, `:set-color` (recolors the filter named by
  `tab.filter.filter_context`, threaded through a new `Effect::
  ExecuteCommand.filter_context` field), `:sidebar-position` (the
  sidebar actually moves sides, with its border flipping to match),
  `:schema <name>`/`:schema none`/`:schema` (shows the current one via a
  status message) — doesn't auto-load a format's default filter file the
  way the TUI's does, see above; `Ctrl+P` file-switcher popup; `:tail`
  (toggles `tail_mode` and jumps to the last line once — doesn't yet
  follow newly-arriving lines, since the GUI has no live-tail growth
  consumption at all)
- Resizable, content-auto-fit log table columns; Time/Level columns hidden
  for unstructured files
- Visual-line selection highlighting; a separate cursor-row highlight for
  plain navigation outside an active selection
- Theme-aware rendering
- Focus handling (window stays reliably focused for key capture)
