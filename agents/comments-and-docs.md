# Comments and documentation

- Do not add comment-based section dividers (`// --- helpers ---`,
  `// === Section Name ===`). Code structure should be self-explanatory.
- Add a comment only when the rationale genuinely cannot be inferred from
  the code itself (a hidden constraint, a non-obvious design decision, a
  workaround). When a comment is warranted, keep it to a short phrase or a
  single line rather than a paragraph. When editing code that already has
  an overly long comment, tighten it rather than leaving it as is.
- Do not justify a reuse or refactor decision in a comment (e.g. "reused
  from X rather than Y because...") or preview future usage (e.g. "drives
  feature A, B, C") on a struct field or type. That content belongs in a
  commit message, not in code.
- Keep type definitions at the top of the file (see `coding-standards.md`).

## User-facing documentation

Do not reference implementation details (crate names such as `regex` or
`aho-corasick`, library names, or internal architecture) in user-facing
documentation. Describe behavior and syntax only; users care about what a
feature does, not how it is implemented.

## CHANGELOG.md

Each entry should be a single, short, plain sentence describing the
user-visible effect, not a multi-clause explanation with parenthetical
root-cause detail. That level of detail belongs in the commit message, not
the changelog.
