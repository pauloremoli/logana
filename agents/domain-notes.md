# Domain notes

## Purpose of highlight mode

The `:highlight` filter type and the `H` highlight-mode toggle are not a
preview or testing feature for filters (i.e. not a way to spot-check what
a filter would match before committing to hiding lines with it). Their
actual purpose is contextual analysis: once include/exclude filters have
narrowed the log to matches of interest, the surrounding lines that
explain why an event occurred are frequently absent from the filtered
view. Highlight mode displays the full, unfiltered log with filter colors
still applied to matches, allowing the user to read genuine context around
the events under investigation without losing track of the original
matches. Describe it in documentation and related UI work as surfacing
full context while retaining match highlighting, not as a filter
preview/test mechanism.
