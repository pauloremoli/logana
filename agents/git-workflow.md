# Git and workflow

## Commit message style

Default format for this repository: a short, plain title line, a blank
line, then a one-to-three sentence high-level description of what changed
(not the rationale or backstory), wrapped at 80 characters per line. Avoid
a title-only message and avoid a detailed per-file bullet list; aim for
one concise paragraph describing the overall change. When combining
several sub-changes into a single commit, describe the feature the commit
represents rather than enumerating each fix folded into it.

Omit the `Co-Authored-By: Claude ...` trailer unless a current
session-level attribution directive explicitly requires it; such
directives take precedence over this default.

## Working through an open-ended backlog

For a broad, multi-item task (e.g. continuing a port plan or working
through `GUI_PARITY_BACKLOG.md`), proceed to the next highest-priority
remaining item rather than pausing after each chunk to ask what to do
next, committing between chunks. Prioritize in this order:

1. Bounded, low-risk items with an established pattern to reuse.
2. Items requiring a new architectural decision (new `Effect` variants,
   async dialogs, OS integration).
3. Items with a known, previously documented architectural obstacle.

Genuine architecture decisions should still be flagged inline (via a
committed doc comment or backlog note) rather than resolved by silent
assumption. Routine prioritization, however, does not require sign-off
before proceeding.

## Persistent backlog tracking

When a plan defers a substantial number of items to future phases,
maintain a tracking file in the repository itself (e.g.
`GUI_PARITY_BACKLOG.md`) in addition to any session-scoped plan. The
repository file persists as project knowledge across sessions; the plan
file does not. Updating the backlog file is part of completing each
subsequent phase.

## Investigating ambiguous bug reports

An initial bug report (e.g. "filter shows a count but no visible lines")
often omits the actual trigger. Before pursuing other hypotheses (Unicode
handling, incremental-filter fast paths, etc.), check specifically whether
a multiline custom schema is involved and whether the schema or raw mode
was changed after filters were already active — this class of symptom is
characteristic of multiline/continuation handling interacting with format
changes. Once static code review stalls, an interactive reproduction
(e.g. via a pty) is generally more productive than continuing to reason
from the code alone.
