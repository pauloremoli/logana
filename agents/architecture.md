# TUI/GUI architecture rules

Shared modules (`shared`, `style`) must not import terminal (`ui`,
crossterm) or GUI (`gui`, GPUI) libraries — see `CONTRIBUTING.md` for the
module map. The rules below codify constraints discovered while enforcing
that separation.

## No stderr writes once the TUI has entered raw mode

Do not use `eprintln!`, `eprint!`, or any direct stderr write from a code
path that can execute after the terminal has entered raw/alt-screen mode
(set up in `main.rs`, before `build_app()`, and held for the remainder of
the session). ratatui owns the entire screen buffer at that point, so a
stderr write corrupts the rendered display rather than printing a clean
line.

Any error that can surface while the TUI is running must be routed through
the application's existing error-reporting channels: `App::session.
startup_warnings` (persistent banner) or `TabState::set_notification`
(transient per-tab banner). If a function's error is already returned as a
`Result` to its caller, the caller — not an inline print — is responsible
for deciding how to surface it.

## No raw OS threads inside a `tokio::spawn_blocking` closure

Do not spawn a second, non-Tokio-managed OS thread (e.g. `std::thread::
scope` with `scope.spawn`) inside a `tokio::task::spawn_blocking` closure
in this application, even when cancellation and channel-drop semantics
appear correct and pass isolated tests.

This pattern previously caused an intermittent hang on process exit that
did not reproduce under code review, unit tests, or pty-driven tests
against a large (3.5 GB) file. Because `main()` has no `process::exit()`
shortcut on the clean-quit path, a stuck `spawn_blocking` closure blocks
the entire process on Tokio runtime drop, requiring a forced kill. The
resolution was a full revert to a single-threaded implementation; the
underlying cause was never conclusively isolated.

If I/O or compute pipelining inside a `spawn_blocking` closure is attempted
again, prefer designs that avoid introducing a thread with independent
join-on-drop semantics (e.g. a bounded prefetch queue with a hard
timeout), and validate with a release build on real hardware before
considering the change complete — isolated and synthetic testing proved
insufficient to catch this class of failure previously.

## GUI work must match the TUI's control-flow shape, not just move off-thread

Backgrounding a GUI operation (e.g. with `tokio::spawn_blocking`) is not
equivalent to adopting how the TUI solves the same problem. The file-open
path is the reference case.

The TUI never performs a read-then-index-then-show sequence, even in the
background. It renders a small preview immediately
(`FileReader::from_file_head`, `TabState::new()` on that preview) and
indexes the full file progressively in the background, swapping in the
complete reader via plain field mutation on the already-visible tab once
indexing finishes — preserving any marks, scroll position, or filters set
during the preview window.

Design constraints distilled from resolving a GUI file-loading performance
issue, in order of increasing specificity:

1. Moving work off the redraw thread (`spawn_blocking`) is necessary but
   not sufficient — if the tab still only appears once the entire file is
   read, the operation remains perceptibly slow regardless of threading.
2. The GUI must adopt the TUI's two-phase shape: an immediate preview tab,
   followed by a background swap-in, each phase driven by its own
   message/task pair.
3. Every code path touching a stated invariant (e.g. "no processing on the
   UI thread") must be audited, not just the path that prompted the
   original report — a synchronous `refresh_visible()` call left in the
   "fully loaded" handler was an O(file-size) regression hiding behind an
   otherwise-correct two-phase implementation.
4. Reproduction attempts should exercise the real code path end to end,
   including OS-level integration points (e.g. `rfd::pick_file()`) that
   are easy to bypass in a synthetic repro. The eventual root cause was
   that `load_file`/`load_full_file` remained a parallel, hand-maintained
   reimplementation of `App::open_file` rather than the TUI's actual code
   path. The fix extracted `build_preview_tab` into
   `src/ingestion/loading.rs` so both frontends call the identical
   function, letting the TUI's existing test suite act as a regression
   guard for the GUI as well.

General principle: when a reported issue persists across multiple fix
attempts, treat each recurrence as new information that narrows the
problem, not as an unrelated follow-up request. Each iteration should move
toward more shared code and less bespoke logic, not the reverse.
