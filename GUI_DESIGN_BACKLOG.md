# GUI design improvements

The native GUI design review began on 2026-10-09. Logs, Bookmarks, and Notes
remain in the footer selector. GUI layouts may evolve independently of the
terminal interface; parsing, filtering, persistence, and loading logic remain
shared where the repository already provides it.

Implement and verify one improvement, then commit it before starting the next.
Native interaction tests verify behavior; fresh desktop screenshots are still
needed to assess the finished layouts visually.

## Implemented

- [x] Name icon controls and inputs for accessibility.
- [x] Scroll sidebar keyboard selections into view.
- [x] Clarify immediate View settings and preserve editor drafts.
- [x] Distinguish static files, paused streams, and active streams; expose Follow.
- [x] Keep errors visible until dismissed and allow copying details.
- [x] Show search counts, scope, empty results, and an explicit clear action.
- [x] Provide actionable start and empty log states.
- [x] Keep source tabs in a bounded, horizontally scrolling row.
- [x] Resize the sidebar and preserve log space in narrow windows.
- [x] Expose Notes in the footer and make note deletion undoable.
- [x] Name editor outcomes and present validation consistently.
- [x] Present structured source information and copy its path.
- [x] Organize preferences into searchable native settings pages.
- [x] Separate sidebar Rules, Fields, and Groups tasks and clarify filtering state.
- [x] Group the Actions catalog by task while preserving search relevance.
- [x] Search the open-file switcher by name or path and show active-file status.
- [x] Group and search keyboard help.
- [x] Search container/device suggestions and show IDs or endpoints.
- [x] Group OpenTelemetry transport choices.
- [x] Use searchable schema and export-template selectors.
- [x] Explain export scope and the destination-selection step.
- [x] Search default-filter mappings and show paths beneath schema names.
- [x] Preview themes using representative log text.
- [x] Separate automatic filter styling from explicit regeneration.
- [x] Compact background tasks into expandable progress details.
- [x] Show paths, row counts, and format information in merge selection.
- [x] Give the file picker a primary Open action and real mixed selection states.
- [x] Preview visible column order and clarify bulk-action scope.
- [x] Group color controls and show enabled/disabled text samples.
- [x] Use native filter progress and bound footer status text.
- [x] Reveal filtered-out note rows without changing individual filter rules.
- [x] Group timestamp comparisons and label clock-time versus full-date conditions.

## Remaining review improvements

- [x] Search Notes while keeping note attachment and navigation source-based.
- [x] Group application menu actions with native separators.
- [x] Page high-cardinality field values while searching across every page.
- [x] Search expanded archive/directory entries while preserving lazy expansion and selection.
- [x] Add a structured/raw row inspector with the existing copy and annotation actions.
- [x] Offer column drag reordering alongside accessible keyboard controls.
- [x] Show a bounded parsed sample when selecting a schema.
- [x] Show a bounded export preview using the existing export renderer.
- [ ] Review final layouts in the running desktop application at narrow and wide sizes.

Preview generation must use bounded samples and the existing parser/export code.
If it needs background work, keep results tied to the current editor and source
so stale completions cannot replace newer selections. Archive search must retain
unknown lazy entries rather than implying their contents were already searched.

## Validation

Run `cargo test` and `cargo clippy --all-targets --all-features` for each change,
plus focused GUI-native checks for controls that change. Loopback-server tests
require execution outside the restricted sandbox. All-feature checks do not
replace GUI-only checks because frontend compilation depends on feature selection.

Schema previews parse at most eight source rows of up to 64 KiB each in a
background task. Editor, source, and selected-schema guards discard stale results.

Export previews sample up to three notes, eight rows per note, and eight
bookmarks. Rendering uses the source reader and original row numbers in a
background task; editor/source/value guards reject stale results.

## Final review record

The inspector's controls fit 600×740 and 1440×1000 windows in the light and
dark built-in themes. Closing its source dismisses the inspector. Native
interaction tests cover archive search and preserved selection, raw copying,
annotation attachment, schema choices, and updated export document fields.

The running GUI was launched with an isolated temporary profile and sample log.
GNOME denied automated window capture (`ScreenshotWindow is not allowed`), so
fresh screenshot-based inspection remains pending. Geometry and interaction
checks do not substitute for that visual review.

## Loading scrollbar regression

- [x] Preserve offscreen row height estimates after table width changes.
- [x] Navigate to the bottom of the last loaded row with End.

Native layout regression coverage replaces a 100-row preview with 2,000 rows
and refits the table width, then checks the scrollbar extent, scrollbar bottom
navigation, and End navigation after rendering.

The row inspector opens from the row context menu; its standalone button was
removed at the user’s request.

## Named DLT devices

- [x] Save named DLT devices from the connection dialog and reuse them later.
- [x] Carry a selected device name into its connected tab title.

Saving uses a background effect and the shared configuration persistence helper.
The connection endpoint remains the source identity for reconnection and saved
file context; the device name supplies the visible tab title.

The DLT dialog keeps a searchable, scrollable saved-device list on the left
and editable connection details on the right. Selecting a saved device fills
the details without modifying its saved values.

## GUI background processing

- [x] Discover filter field names and observed values in a background task.
- [x] Compile GUI filter expressions and prepare field/date conditions in workers.
- [x] Finalize multiline, bookmark, collapse, and unfiltered views in workers.
- [x] Move rendering field discovery and column fitting out of render callbacks.
- [x] Validate regex and timestamp filter patterns in background tasks.
- [x] Move schema-dependent maps out of UI handlers.
- [ ] Move continuation display commands out of UI handlers.

Field suggestions preserve editor drafts and reject results from a superseded
editor, parser generation, or file snapshot.

Filter-file results and direct filter edits start cancellable background preparation.
Marks-only and style-only refreshes use the same worker preparation path.

GUI refreshes retain the current view while computing replacements. Completed
views arrive ready to install; GUI scans avoid restoring or duplicating large
visible-index caches on the UI thread. Progressive index capacity is allocated
in the worker to prevent full-vector reallocations during frame updates.

Table preparation runs against captured readers and parsers. Rendering reads
prepared metadata; delayed results preserve manual column resizing and column
editor choices. Tab switches schedule fitting without reading file contents.

Pattern validation keeps the editor open, displays checking feedback, preserves
validation errors, and ignores results for changed or closed editors. Saving
starts only after the current pattern passes validation.

Schema changes build parser metadata against a captured source in the background.
Results for changed readers restart preparation; closed tabs cannot receive them.
