# DLT Streaming

logana can connect to a running DLT daemon over TCP and stream log messages in real time, with the same filtering, search, and annotation features available for file-based logs.

## Opening a DLT Stream

From normal mode, type:

```
:dlt
```

A picker lists configured DLT devices. Navigate with `j` / `k` and press `Enter` to connect. The stream opens in a new tab.

| Key | Action |
|---|---|
| `j` / `k` | Navigate device list |
| `Enter` | Connect to selected device |
| `a` | Add a new device inline |
| `Esc` | Cancel |

## Configuring Devices

DLT devices can be configured in `~/.config/logana/config.json`:

```json
{
  "dlt_devices": [
    { "name": "local", "host": "127.0.0.1", "port": 3490 },
    { "name": "target-ecu", "host": "192.168.1.100", "port": 3490 }
  ]
}
```

The default port is 3490. Devices can also be added from the selection panel by pressing `a`.

## Opening DLT Binary Files

DLT binary files (`.dlt`) are opened like any other log file:

```sh
logana trace.dlt
```

Three binary layouts are detected automatically: storage format (with `DLT\x01` magic), wire format (concatenated messages without storage headers), and simplified format.

## Auto-Reconnect

If the connection to the DLT daemon fails or drops, logana retries automatically with increasing backoff (0s, 2s, 5s, 10s). The tab name shows `[RETRY #N]` while reconnecting, and the error details appear in the status bar. Once the connection is re-established, streaming resumes normally.

Session-restored DLT tabs also reconnect automatically without blocking the UI.

## Session Persistence

DLT tabs are persisted across sessions. When you reopen logana, it reconnects to any DLT daemons that were open in the previous session. The source identifier stored in the session database is `dlt://host:port`.

## Tail Mode

DLT streams benefit from tail mode — when enabled, the view auto-scrolls to show new log entries as they arrive:

```
:tail
```

## Fields

DLT messages expose the following fields for filtering and display:

| Field | Description |
|---|---|
| `timestamp` | Wall-clock time (streaming) or relative time (file) |
| `hw_ts` | Hardware timestamp counter |
| `mcnt` | Message counter (0-255) |
| `ecu` | ECU identifier |
| `apid` | Application ID (shown as target) |
| `ctid` | Context ID |
| `type` | Message type (log, trace, network, control) |
| `subtype` | Sub-type (fatal, error, warn, info, debug, verbose) |
| `mode` | Verbose or non-verbose |
