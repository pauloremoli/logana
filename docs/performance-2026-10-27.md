# GUI cache and pipeline measurements — 2026-10-27

## Scope and method

Measurements use synthetic logs on an AMD Ryzen 9 8945HS (16 logical CPUs),
Rust 1.99.0, and the repository's optimized release profile. GUI measurements
use GPUI's headless test window at 1200 × 900; they include element construction,
layout, and painting into the test scene, but not a real display's GPU or
presentation latency. These timings are not a frame-rate guarantee for real logs.

CPU sampling with `perf` is unavailable on this host (`perf_event_paranoid=4`).
The results below use repeatable phase timings, Criterion microbenchmarks, and
parser/measurement counters instead of sampled CPU stacks.

The GUI fixture has 2,000 JSON records, with either 80 or 10,000 message
characters, no active filters, and the sidebar hidden. Warm frames are measured
in batches of 20. Selection frames dispatch the real `SelectLog` message and
then render. Source construction and schema detection are outside the warm
frame timing. Parsing and text-filter timings below are separate phases.

## Changes

- One bounded GUI cache shares displayed field values and parsed level/target
  metadata between cells, context-menu preparation, and horizontal sizing.
- Measured message widths are cached per source row and font size. A viewport
  cache reuses the assembled column widths on unchanged renders and horizontal
  scrolling. Moving vertically prepares newly encountered rows.
- Value-color detector ranges are reused. Disabling a detector invalidates its
  ranges; changing a color resolves the current theme without rerunning regexes.
  The redundant second sort/split of already-disjoint ranges was removed.
- Cache identity includes the tab, parser generation/identity, displayed columns,
  raw/key display options, and source snapshot. Source replacement and incomplete
  line appends invalidate it even when the line count stays unchanged.
- Limits are 1,024 rows and 32 MiB of displayed text. Wrapping/detector ranges and row
  metadata add memory proportional to those bounds. The cache holds a weak line
  index identity, not the file's backing buffer, so it does not force full-buffer
  copies when a live source appends.

Warm renders still perform cache-key checks, UI construction/layout/painting,
filter-highlight projection, and dynamic search/selection work. Cold rows and
invalidated entries require initial parsing, text measurement, and detection.
This change removes repeated expensive preparation, not the rendering itself.

## GUI before and after

The baseline is the working tree with the scrollbar and stale-test fixes, before
presentation caching. [Raw timing samples](performance-2026-10-27.json) record all
three comparison runs in milliseconds.

GUI timings are medians of three alternating before/after process runs. Each
warm/selection sample averages 20 frames. The 30-filter probe averages 50
selection events with 466 matching visible rows and its filter sidebar shown.

| Fixture / operation | Before | After | Reduction |
| --- | ---: | ---: | ---: |
| 80-character messages: warm frame | 2.203 ms | 2.004 ms | 9.0% |
| 80-character messages: selection + frame | 2.078 ms | 1.949 ms | 6.2% |
| 10,000-character messages: first frame | 10.889 ms | 9.951 ms | 8.6% |
| 10,000-character messages: warm frame | 5.382 ms | 3.213 ms | 40.3% |
| 10,000-character messages: selection + frame | 5.297 ms | 3.039 ms | 42.6% |
| 30-filter sidebar probe: selection + frame | 22.525 ms | 22.463 ms | 0.3% |

The long-message warm frame improves by about **40%**, and selection plus
rendering by about **43%**. The 30-filter sidebar case is effectively unchanged
(0.3%, within timing noise), so this cache does not solve that separate cost.

Standalone parsing remains comparable: 2,000 short JSON rows take 0.349 ms
before and 0.356 ms after; long rows take 8.534 ms before and 8.764 ms after.
The change reduces redundant parsing in presentation, not parser throughput.

The regression counter found **21 parser calls over three unchanged renders**
of a single message-row fixture before caching. The finished cache test asserts
**zero**, including the default level/target columns and a subsequent font-size
change. A second counter verifies horizontal scrolling reuses measured widths.

## Parsing, filtering, and shared render helpers

Criterion estimates below use 10 samples, 0.2 s warmup, and 0.5 s measurement.
These isolate shared helpers; they are not total GUI frame times.

| Operation | Estimate |
| --- | ---: |
| JSON parsing/timestamp extraction, 10,000 records | 2.640 ms |
| Field-name discovery, 200 JSON records | 61.407 µs |
| Clone cached field names | 68 ns |
| Raw visibility scan, 100,000 lines, no filters | 0.387 ms |
| Raw visibility scan, 100,000 lines, one include | 0.626 ms |
| Raw visibility scan, 100,000 lines, one exclude | 0.655 ms |
| Raw visibility scan, 100,000 lines, five filters | 0.830 ms |
| Shared per-line render helper, no filters | 773 ns |
| Shared per-line render helper, five filters | 967 ns |
| Five-filter evaluation alone | 135 ns |
| Value-color detection alone | 715 ns |
| Shared styled-span rendering alone | 55 ns |

The no-filter raw visibility benchmark materializes a vector of indices; the
application's unfiltered `VisibleLines::All` representation avoids that allocation.
Raw visibility benchmarks do not include date/field filtering, filter preparation,
match-count collection, continuation handling, or progressive GUI publication.

Value-color detection accounts for about 92% of the sampled no-filter per-line
helper cost. This supports caching detector ranges rather than changing the text
filter algorithm to address redraw slowness. Existing background filter workers,
compiled matchers, filter-result reuse, and deferred schema/field discovery are
preserved.

## Oversized wrapped-message follow-up

The timing table above covers unwrapped messages. A later wrapped-message
regression exposed a separate cost: creating one text element for every visual
line of a message, including thousands of lines outside the viewport. The height
calculation and cell rendering also repeated the full wrapping pass.

Wrapped cells now cache Unicode-safe byte ranges and construct text elements
only for the intersection of their final layout bounds with the list viewport.
The complete row height is retained, so the scrollbar still represents the
whole message. Selection endpoints retain the original message byte offsets.

The regression uses a 160,003-byte Unicode message in a 700 × 500 window and
prepares **11 text chunks** for a warm viewport. It checks scrolling inside the
message, mouse-wheel events, copying after scrolling, reaching the final byte,
and keyboard navigation to the following message and back. A separate test
checks wrapping-cache reuse and invalidation when column capacity changes.
This is a deterministic bound on render work, not a new wall-clock speed claim.

## Remaining opportunities

1. Measure long-message text layout/painting and dynamic highlight projection
   separately on a real display. Those still run after these cache hits and are
   the next likely sources of long-message frame time.
2. Isolate sidebar construction and filter-highlight composition in the existing
   30-filter click probe. Raw filtering scans alone do not explain total GUI
   event/render latency; do not attribute that difference to the filter engine
   without another measurement.
3. Profile structured date/field scans with representative schemas and regex
   filters before adding a file-wide parsed-record cache. The current bounded
   presentation cache intentionally avoids retaining every parsed row of a huge
   file.

## Reproduction

```sh
cargo test --release --no-default-features --features gui-test --lib profile_gui_pipeline -- --ignored --nocapture
cargo test --release --no-default-features --features gui-test --lib perf_probe_click_select_row -- --nocapture
cargo bench --bench render_pipeline -- 'render_pipeline/(date_filter_parse/single_pass/10000$|per_line|collect_field_names)' --sample-size 10 --warm-up-time 0.2 --measurement-time 0.5
cargo bench --bench visible_lines -- 'visible_lines/compute_visible.*/100000$' --sample-size 10 --warm-up-time 0.2 --measurement-time 0.5
```

Validation: `cargo test`, `cargo test --no-default-features --features gui-test
--lib`, and `cargo clippy --all-targets --all-features` pass. Cache regressions
cover warm parsing, repeated width measurement, zoom, partial appends, same-size
source replacement, display/schema invalidation, bounded row count, and detector
settings/theme changes.
