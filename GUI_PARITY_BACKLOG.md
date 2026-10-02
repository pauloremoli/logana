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
- [ ] Theming/coloring: `SetTheme`, `Theme`, `LevelColors`, `ValueColors`,
      `SetColor`
- [ ] Field layout: `HideField`, `ShowField`, `ShowAllFields`,
      `SelectFields` (needs structured/JSON field display, which the GUI
      doesn't have yet)
- [ ] Sources: `Merge`, `Docker`, `Dlt`, `Otel`, `Schema`
- [ ] Streaming: `Tail`, `Reset` (`Pause`/`Resume`/`Stop` already work, see
      above)
- [ ] Misc: `EnableMcp`/`DisableMcp`, `DefaultFilters`, `SidebarPosition`,
      `Run`, `Path`

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
- [ ] `file_switcher_mode`
- [ ] `merge_select_mode`
- [ ] `select_fields_mode`
- [ ] `theme_picker_mode`
- [ ] `value_colors_mode`
- [ ] `visual_char_mode` — only visual **line** selection is highlighted in
      `log_pane.rs`; char-wise visual selection has no rendering

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
  `SearchMode`), time-range dropdown, `?` keybindings-help overlay
- Resizable, content-auto-fit log table columns; Time/Level columns hidden
  for unstructured files
- Visual-line selection highlighting; a separate cursor-row highlight for
  plain navigation outside an active selection
- Theme-aware rendering
- Focus handling (window stays reliably focused for key capture)
