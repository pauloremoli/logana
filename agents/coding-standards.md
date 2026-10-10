# Coding standards

From `.claude/CLAUDE.md`:

- At least 80% test coverage.
- RED/GREEN TDD: write the failing test first, then the code that passes it.
- Apply SOLID principles when designing code.
- Keep functions small.
- Keep type definitions at the top of the file.
- Hold code to a senior/principal-engineer review bar: do not let through
  anything that would not pass a rigorous code review.

## Reuse over reimplementation

Before writing new logic for the GUI, check whether the TUI already has it.
If it exists but is private or coupled to `&mut App`/ratatui rendering, do a
small refactor to extract/share it (bump visibility, pull the pure sub-piece
into a shared module) rather than duplicating it in `src/gui/`.

- Pure logic, wrong visibility → bump to `pub`/`pub(crate)`, import directly.
- Logic mixed into ratatui-rendering or `&mut App`-coupled code → extract
  just the pure sub-piece into a `pub fn` in a shared, neutral location.
- Only write new GUI-local logic when nothing equivalent exists in the TUI.

This also applies to matching *shape*, not just functions: if the TUI solves
a problem (e.g. progressive file loading — instant preview tab, background
full-index swap-in) with a particular control-flow pattern, port that
pattern, don't just background the GUI's existing one-shot version and call
it reused. See `architecture.md` for the concrete case this was learned from.

## Frontend-agnostic core, no generics

When interaction logic (e.g. `Mode::handle_key`, `config::Keybindings`) needs
to be shared between the TUI (crossterm-typed) and the GUI, do not import one
frontend's input crate into the other (no crossterm types inside `src/gui/`).

Also do not reach for generics on the shared trait as the fix: if the
abstraction is used as a trait object (`Box<dyn Mode>`, needed here for
dynamic mode switching), a generic method breaks object-safety, and
parameterizing the whole trait cascades into every type that stores
`Box<dyn Mode<K>>`.

Instead, define one small frontend-agnostic type (e.g.
`crate::input::{KeyCode, KeyModifiers}`) that the shared core operates on.
Each frontend converts its own native event type into the neutral one at
exactly one boundary point. If the neutral type's shape mirrors whichever
frontend already uses it natively, adopting it is a one-line `use` swap per
call site instead of a rewrite.
