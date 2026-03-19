# OTel Collector

logana can receive OpenTelemetry logs in real time over gRPC or HTTP/JSON, turning it into a live OTel log viewer with the same filtering, search, and annotation features available for file-based logs.

## Starting a Receiver

From normal mode, type:

```
:otel              # gRPC on port 4317 (default — matches OTel SDK defaults)
:otel --http       # HTTP/JSON on port 4318
:otel 4317         # gRPC on a custom port
:otel --http 4318  # HTTP/JSON on a custom port
```

The receiver opens in a new tab and listens for incoming log export requests. Logs appear as they arrive.

## Transport Modes

| Mode | Command | Default Port | Protocol |
|---|---|---|---|
| gRPC | `:otel` | 4317 | OTLP/gRPC (protobuf) |
| HTTP/JSON | `:otel --http` | 4318 | OTLP/HTTP (JSON or protobuf) |

### gRPC (default)

The gRPC receiver accepts `ExportLogsServiceRequest` messages on port 4317. This matches the default export protocol used by most OTel SDKs.

The server runs in **plaintext mode** (no TLS). Configure your SDK to use an insecure connection:

```sh
# Environment variable (works for all OTel SDKs)
OTEL_EXPORTER_OTLP_INSECURE=true

# Or use the http:// scheme in the endpoint URL
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317
```

### HTTP/JSON

The HTTP receiver accepts `POST /v1/logs` with `application/json` or `application/x-protobuf` content types, and handles gzip-compressed request bodies.

## Auto-Reconnect

If the receiver encounters an error on startup (e.g. port already in use), logana reports the error in the tab. Fix the conflict and reopen with `:otel` again.

## Session Persistence

OTel collector tabs are persisted across sessions. When you reopen logana, it automatically restarts the receiver on the same port. The source identifier stored in the session database is `otlp-grpc://<port>` (gRPC) or `otlp://<port>` (HTTP).

## Parsed Fields

Logs received over OTLP are parsed with the same OTel parser used for file-based OTLP logs:

| Field | Source |
|---|---|
| Timestamp | `timeUnixNano` |
| Level | `severityNumber` / `severityText` |
| Message | `body.stringValue` |
| Target | `service.name`, `code.namespace`, `logger` (from resource or log attributes) |
| Extra fields | All other resource attributes and log attributes |
