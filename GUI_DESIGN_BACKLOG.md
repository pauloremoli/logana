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
- [ ] Search archive/directory entries while preserving lazy expansion and selection.
- [ ] Add a structured/raw row inspector with the existing copy and annotation actions.
- [ ] Offer column drag reordering alongside accessible keyboard controls.
- [ ] Show a bounded parsed sample when selecting a schema.
- [ ] Show a bounded export preview using the existing export renderer.
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
