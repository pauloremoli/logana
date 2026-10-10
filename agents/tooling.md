# Tooling

## CodeGraph

This repository is indexed by CodeGraph (`.codegraph/` at the repository
root). Prefer it over grep/find or direct file reads when locating or
understanding code:

- `codegraph_explore` (MCP tool, when available) answers most code
  questions in a single call, returning the relevant symbols' verbatim
  source, the call paths between them (including dynamic-dispatch hops
  that grep cannot follow), and a summary of their dependents. Include a
  file or symbol name in the query to retrieve its current, line-numbered
  source.
- `codegraph explore "<symbol names or question>"` (shell) provides the
  same output when the MCP tool is unavailable.

A single call typically answers the full question. Treat its output as
equivalent to an already-performed file read rather than reading the same
file again.
