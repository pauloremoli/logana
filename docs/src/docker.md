# Docker Logs

logana can stream logs from any running Docker container directly in the terminal, with the same filtering, search, and annotation features available for file-based logs.

## Opening a Container Stream

From normal mode, type:

```
:docker
```

A picker lists all running containers. Navigate with `j` / `k` and press `Enter` to attach. The stream opens in a new tab.

| Key | Action |
|---|---|
| `j` / `k` | Navigate container list |
| `Enter` | Attach to selected container |
| `Esc` | Cancel |

## Session Persistence

Docker tabs are persisted across sessions. When you reopen logana, it automatically re-attaches to any Docker containers that were open in the previous session, by container name. The source identifier stored in the session database is `docker:<container-name>`.

## Tail Mode

Docker tabs benefit from tail mode — when enabled, the view auto-scrolls to show new log entries as they arrive:

```sh
:tail     # toggle tail mode on/off
```

When tail mode is active, `[TAIL]` appears in the log panel title.

## Filtering and Annotations

All filter, search, and annotation features work identically for Docker streams. Filters are persisted per container name, just like file-based logs.

## Piping Docker Compose Logs

You can also pipe `docker compose logs` directly into logana:

```sh
docker compose logs -f 2>&1 | logana
```

The `2>&1` redirect is important — without it, Docker's warnings (e.g. unset variable notices) go straight to the terminal and corrupt the TUI display. Merging stderr into stdout ensures everything flows through the pipe and appears as log entries inside logana, where you can filter them as needed.

To suppress the warnings entirely instead:

```sh
docker compose logs -f 2>/dev/null | logana
```

## Requirements

- Docker must be installed and accessible via `docker` in `PATH`.
- The `docker ps` command must return running containers.
- Logs are streamed via `docker logs -f <container-id>`, with stdout and stderr merged.
