use std::io;
use std::sync::Arc;

use tokio::sync::watch;

pub async fn spawn_otlp_http_receiver(
    port: u16,
) -> io::Result<(watch::Receiver<()>, tempfile::NamedTempFile)> {
    let temp_file = tempfile::NamedTempFile::new()?;
    let temp_path = Arc::new(temp_file.path().to_owned());
    let (tx, rx) = watch::channel(());
    let tx = Arc::new(tx);

    let app = build_otlp_router(tx.clone(), temp_path);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::AddrInUse, e))?;

    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Ok((rx, temp_file))
}

fn build_otlp_router(
    tx: Arc<watch::Sender<()>>,
    temp_path: Arc<std::path::PathBuf>,
) -> axum::Router {
    axum::Router::new().route(
        "/v1/logs",
        axum::routing::post(
            move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                let tx = tx.clone();
                let path = temp_path.clone();
                async move {
                    let content_type = headers
                        .get(axum::http::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    let is_protobuf = content_type.contains("application/x-protobuf");

                    let content_encoding = headers
                        .get(axum::http::header::CONTENT_ENCODING)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    let raw: Vec<u8> = if content_encoding.contains("gzip") {
                        match decompress_gzip(&body) {
                            Ok(b) => b,
                            Err(_) => return axum::http::StatusCode::BAD_REQUEST,
                        }
                    } else {
                        body.to_vec()
                    };

                    let lines = if is_protobuf {
                        match otlp_protobuf_to_lines(&raw) {
                            Ok(l) => l,
                            Err(_) => return axum::http::StatusCode::BAD_REQUEST,
                        }
                    } else {
                        match serde_json::from_slice::<serde_json::Value>(&raw) {
                            Ok(payload) => otlp_payload_to_lines(&payload),
                            Err(_) => return axum::http::StatusCode::BAD_REQUEST,
                        }
                    };

                    let mut out = Vec::<u8>::new();
                    for line in lines {
                        out.extend_from_slice(line.as_bytes());
                        out.push(b'\n');
                    }
                    if !out.is_empty() {
                        if let Ok(mut f) =
                            std::fs::OpenOptions::new().append(true).open(path.as_ref())
                        {
                            use std::io::Write as _;
                            let _ = f.write_all(&out);
                            let _ = f.flush();
                        }
                        let _ = tx.send(());
                    }
                    axum::http::StatusCode::OK
                }
            },
        ),
    )
}

fn decompress_gzip(data: &[u8]) -> io::Result<Vec<u8>> {
    use flate2::read::GzDecoder;
    use std::io::Read as _;
    let mut decoder = GzDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

pub(crate) fn otlp_payload_to_lines(payload: &serde_json::Value) -> Vec<String> {
    let mut lines = Vec::new();
    let Some(resource_logs) = payload.get("resourceLogs").and_then(|v| v.as_array()) else {
        return lines;
    };
    for resource_log in resource_logs {
        let resource_attrs = otlp_extract_resource_attrs(resource_log);
        let Some(scope_logs) = resource_log.get("scopeLogs").and_then(|v| v.as_array()) else {
            continue;
        };
        for scope_log in scope_logs {
            let Some(log_records) = scope_log.get("logRecords").and_then(|v| v.as_array()) else {
                continue;
            };
            for record in log_records {
                if let Some(obj) = record.as_object() {
                    let mut merged = obj.clone();
                    for (k, v) in &resource_attrs {
                        merged.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                    if let Ok(line) = serde_json::to_string(&merged) {
                        lines.push(line);
                    }
                }
            }
        }
    }
    lines
}

fn otlp_extract_resource_attrs(
    resource_log: &serde_json::Value,
) -> serde_json::Map<String, serde_json::Value> {
    resource_log
        .get("resource")
        .and_then(|r| r.get("attributes"))
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| {
                    let key = entry.get("key")?.as_str()?.to_string();
                    let raw_val = entry.get("value")?;
                    let val = raw_val
                        .get("stringValue")
                        .and_then(|s| s.as_str())
                        .map(|s| serde_json::Value::String(s.to_string()))
                        .unwrap_or_else(|| raw_val.clone());
                    Some((key, val))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn any_value_to_otlp_json_wrapper(
    v: &opentelemetry_proto::tonic::common::v1::AnyValue,
) -> serde_json::Value {
    use opentelemetry_proto::tonic::common::v1::any_value::Value;
    match &v.value {
        Some(Value::StringValue(s)) => serde_json::json!({"stringValue": s}),
        Some(Value::IntValue(i)) => serde_json::json!({"intValue": i.to_string()}),
        Some(Value::DoubleValue(d)) => serde_json::json!({"doubleValue": d}),
        Some(Value::BoolValue(b)) => serde_json::json!({"boolValue": b}),
        Some(Value::BytesValue(b)) => serde_json::json!({"stringValue": bytes_to_hex(b)}),
        _ => serde_json::json!({"stringValue": ""}),
    }
}

fn any_value_to_plain(v: &opentelemetry_proto::tonic::common::v1::AnyValue) -> serde_json::Value {
    use opentelemetry_proto::tonic::common::v1::any_value::Value;
    match &v.value {
        Some(Value::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(Value::IntValue(i)) => serde_json::Value::Number((*i).into()),
        Some(Value::DoubleValue(d)) => serde_json::Number::from_f64(*d)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Some(Value::BoolValue(b)) => serde_json::Value::Bool(*b),
        Some(Value::BytesValue(b)) => serde_json::Value::String(bytes_to_hex(b)),
        _ => serde_json::Value::String(String::new()),
    }
}

pub(crate) fn otlp_export_request_to_lines(
    request: &opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest,
) -> Vec<String> {
    let mut lines = Vec::new();

    for resource_logs in &request.resource_logs {
        let resource_attrs: serde_json::Map<String, serde_json::Value> = resource_logs
            .resource
            .as_ref()
            .map(|r| {
                r.attributes
                    .iter()
                    .filter_map(|kv| {
                        kv.value
                            .as_ref()
                            .map(|v| (kv.key.clone(), any_value_to_plain(v)))
                    })
                    .collect()
            })
            .unwrap_or_default();

        for scope_logs in &resource_logs.scope_logs {
            for record in &scope_logs.log_records {
                let mut map = serde_json::Map::new();

                if record.time_unix_nano != 0 {
                    map.insert(
                        "timeUnixNano".into(),
                        serde_json::Value::String(record.time_unix_nano.to_string()),
                    );
                }
                if record.observed_time_unix_nano != 0 {
                    map.insert(
                        "observedTimeUnixNano".into(),
                        serde_json::Value::String(record.observed_time_unix_nano.to_string()),
                    );
                }
                if record.severity_number != 0 {
                    map.insert(
                        "severityNumber".into(),
                        serde_json::Value::Number(record.severity_number.into()),
                    );
                }
                if !record.severity_text.is_empty() {
                    map.insert(
                        "severityText".into(),
                        serde_json::Value::String(record.severity_text.clone()),
                    );
                }
                if let Some(body) = &record.body {
                    map.insert("body".into(), any_value_to_otlp_json_wrapper(body));
                }
                if !record.attributes.is_empty() {
                    let attrs: Vec<serde_json::Value> = record
                        .attributes
                        .iter()
                        .map(|kv| {
                            let val = kv
                                .value
                                .as_ref()
                                .map(any_value_to_otlp_json_wrapper)
                                .unwrap_or(serde_json::json!({"stringValue": ""}));
                            serde_json::json!({"key": kv.key, "value": val})
                        })
                        .collect();
                    map.insert("attributes".into(), serde_json::Value::Array(attrs));
                }
                if !record.trace_id.iter().all(|&b| b == 0) {
                    map.insert(
                        "traceId".into(),
                        serde_json::Value::String(bytes_to_hex(&record.trace_id)),
                    );
                }
                if !record.span_id.iter().all(|&b| b == 0) {
                    map.insert(
                        "spanId".into(),
                        serde_json::Value::String(bytes_to_hex(&record.span_id)),
                    );
                }

                for (k, v) in &resource_attrs {
                    map.entry(k.clone()).or_insert_with(|| v.clone());
                }

                if let Ok(line) = serde_json::to_string(&map) {
                    lines.push(line);
                }
            }
        }
    }

    lines
}

pub(crate) fn otlp_protobuf_to_lines(data: &[u8]) -> Result<Vec<String>, prost::DecodeError> {
    use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
    use prost::Message;

    let request = ExportLogsServiceRequest::decode(data)?;
    Ok(otlp_export_request_to_lines(&request))
}

struct OtlpGrpcHandler {
    tx: std::sync::Arc<tokio::sync::watch::Sender<()>>,
    temp_path: std::sync::Arc<std::path::PathBuf>,
}

#[tonic::async_trait]
impl opentelemetry_proto::tonic::collector::logs::v1::logs_service_server::LogsService
    for OtlpGrpcHandler
{
    async fn export(
        &self,
        request: tonic::Request<
            opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest,
        >,
    ) -> Result<
        tonic::Response<opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceResponse>,
        tonic::Status,
    > {
        let lines = otlp_export_request_to_lines(&request.into_inner());
        let mut out = Vec::<u8>::new();
        for line in lines {
            out.extend_from_slice(line.as_bytes());
            out.push(b'\n');
        }
        if !out.is_empty() {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .append(true)
                .open(self.temp_path.as_ref())
            {
                use std::io::Write as _;
                let _ = f.write_all(&out);
                let _ = f.flush();
            }
            let _ = self.tx.send(());
        }
        Ok(tonic::Response::new(
            opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceResponse {
                partial_success: None,
            },
        ))
    }
}

pub async fn spawn_otlp_grpc_receiver(
    port: u16,
) -> std::io::Result<(tokio::sync::watch::Receiver<()>, tempfile::NamedTempFile)> {
    use opentelemetry_proto::tonic::collector::logs::v1::logs_service_server::LogsServiceServer;

    let temp_file = tempfile::NamedTempFile::new()?;
    let temp_path = std::sync::Arc::new(temp_file.path().to_owned());
    let (tx, rx) = tokio::sync::watch::channel(());
    let tx = std::sync::Arc::new(tx);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::AddrInUse, e))?;
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);

    let handler = OtlpGrpcHandler { tx, temp_path };
    let service = LogsServiceServer::new(handler);

    tokio::spawn(async move {
        let _ = tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming(incoming)
            .await;
    });

    Ok((rx, temp_file))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn otlp_payload(service: &str, records: &[(&str, u32, &str)]) -> serde_json::Value {
        let log_records: Vec<serde_json::Value> = records
            .iter()
            .map(|(ts, sev, body)| {
                serde_json::json!({
                    "timeUnixNano": ts,
                    "severityNumber": sev,
                    "body": {"stringValue": body}
                })
            })
            .collect();
        serde_json::json!({
            "resourceLogs": [{
                "resource": {
                    "attributes": [
                        {"key": "service.name", "value": {"stringValue": service}}
                    ]
                },
                "scopeLogs": [{"logRecords": log_records}]
            }]
        })
    }

    #[test]
    fn test_otlp_payload_to_lines_basic() {
        let payload = otlp_payload(
            "my-svc",
            &[("1000000000", 9, "hello"), ("2000000000", 17, "error!")],
        );
        let lines = otlp_payload_to_lines(&payload);
        assert_eq!(lines.len(), 2);
        for line in &lines {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(v["service.name"], "my-svc");
        }
        let v0: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(v0["timeUnixNano"], "1000000000");
        assert_eq!(v0["severityNumber"], 9);
        assert_eq!(v0["body"]["stringValue"], "hello");
    }

    #[test]
    fn test_otlp_payload_to_lines_resource_attrs_not_overwrite_record() {
        let payload = serde_json::json!({
            "resourceLogs": [{
                "resource": {
                    "attributes": [
                        {"key": "service.name", "value": {"stringValue": "resource-svc"}}
                    ]
                },
                "scopeLogs": [{
                    "logRecords": [{
                        "timeUnixNano": "1",
                        "service.name": "record-svc"
                    }]
                }]
            }]
        });
        let lines = otlp_payload_to_lines(&payload);
        assert_eq!(lines.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(
            v["service.name"], "record-svc",
            "record value should not be overwritten"
        );
    }

    #[test]
    fn test_otlp_payload_to_lines_empty_resource_logs() {
        let payload = serde_json::json!({"resourceLogs": []});
        assert!(otlp_payload_to_lines(&payload).is_empty());
    }

    #[test]
    fn test_otlp_payload_to_lines_missing_key() {
        let payload = serde_json::json!({"notResourceLogs": []});
        assert!(otlp_payload_to_lines(&payload).is_empty());
    }

    #[test]
    fn test_otlp_payload_to_lines_multiple_resource_logs() {
        let payload = serde_json::json!({
            "resourceLogs": [
                {
                    "resource": {"attributes": [{"key": "service.name", "value": {"stringValue": "svc-a"}}]},
                    "scopeLogs": [{"logRecords": [{"timeUnixNano": "1", "severityNumber": 9}]}]
                },
                {
                    "resource": {"attributes": [{"key": "service.name", "value": {"stringValue": "svc-b"}}]},
                    "scopeLogs": [{"logRecords": [{"timeUnixNano": "2", "severityNumber": 17}]}]
                }
            ]
        });
        let lines = otlp_payload_to_lines(&payload);
        assert_eq!(lines.len(), 2);
        let v0: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        let v1: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
        assert_eq!(v0["service.name"], "svc-a");
        assert_eq!(v1["service.name"], "svc-b");
    }

    fn otlp_test_router() -> (
        axum::Router,
        std::sync::Arc<tokio::sync::watch::Sender<()>>,
        tokio::sync::watch::Receiver<()>,
        tempfile::NamedTempFile,
    ) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let temp_path = std::sync::Arc::new(tmp.path().to_owned());
        let (tx, rx) = tokio::sync::watch::channel(());
        let tx = std::sync::Arc::new(tx);
        let app = build_otlp_router(tx.clone(), temp_path);
        (app, tx, rx, tmp)
    }

    #[tokio::test]
    async fn test_spawn_otlp_http_receiver_accepts_logs() {
        use axum::body::Body;
        use http::Request;
        use tower::ServiceExt;

        let payload = otlp_payload("test-svc", &[("1000000000", 9, "hello from test")]);
        let body = serde_json::to_string(&payload).unwrap();

        let (app, _tx, rx, tmp) = otlp_test_router();
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/logs")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert!(rx.has_changed().unwrap());

        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(content.contains("hello from test"));
        assert!(content.contains("test-svc"));
    }

    #[tokio::test]
    async fn test_spawn_otlp_http_receiver_invalid_json_returns_bad_request() {
        use axum::body::Body;
        use http::Request;
        use tower::ServiceExt;

        let (app, _tx, _rx, _tmp) = otlp_test_router();
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/logs")
                    .header("content-type", "application/json")
                    .body(Body::from("not json at all"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    fn make_protobuf_request(service: &str, ts: u64, sev: i32, body_text: &str) -> Vec<u8> {
        use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
        use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue, any_value::Value};
        use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
        use opentelemetry_proto::tonic::resource::v1::Resource;
        use prost::Message;

        let request = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: Some(Resource {
                    attributes: vec![KeyValue {
                        key: "service.name".into(),
                        value: Some(AnyValue {
                            value: Some(Value::StringValue(service.into())),
                        }),
                    }],
                    dropped_attributes_count: 0,
                }),
                scope_logs: vec![ScopeLogs {
                    log_records: vec![LogRecord {
                        time_unix_nano: ts,
                        severity_number: sev,
                        body: Some(AnyValue {
                            value: Some(Value::StringValue(body_text.into())),
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                schema_url: String::new(),
            }],
        };
        request.encode_to_vec()
    }

    #[test]
    fn test_otlp_protobuf_to_lines_basic() {
        let bytes = make_protobuf_request("proto-svc", 1_000_000_000, 9, "proto hello");
        let lines = otlp_protobuf_to_lines(&bytes).unwrap();
        assert_eq!(lines.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(v["timeUnixNano"], "1000000000");
        assert_eq!(v["severityNumber"], 9);
        assert_eq!(v["body"]["stringValue"], "proto hello");
        assert_eq!(v["service.name"], "proto-svc");
    }

    #[test]
    fn test_otlp_protobuf_to_lines_empty() {
        use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
        use prost::Message;
        let bytes = ExportLogsServiceRequest::default().encode_to_vec();
        let lines = otlp_protobuf_to_lines(&bytes).unwrap();
        assert!(lines.is_empty());
    }

    #[test]
    fn test_otlp_protobuf_to_lines_invalid_bytes() {
        assert!(otlp_protobuf_to_lines(b"this is not protobuf garbage!!!").is_err());
    }

    #[test]
    fn test_otlp_protobuf_to_lines_body_format() {
        let bytes = make_protobuf_request("svc", 1, 9, "my message");
        let lines = otlp_protobuf_to_lines(&bytes).unwrap();
        let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert!(v["body"].is_object(), "body must be a JSON object wrapper");
        assert_eq!(v["body"]["stringValue"], "my message");
        assert!(!v["body"].is_string(), "body must not be a plain string");
    }

    #[test]
    fn test_otlp_protobuf_to_lines_attributes_format() {
        use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
        use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue, any_value::Value};
        use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
        use prost::Message;

        let request = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: None,
                scope_logs: vec![ScopeLogs {
                    log_records: vec![LogRecord {
                        time_unix_nano: 1,
                        attributes: vec![KeyValue {
                            key: "http.method".into(),
                            value: Some(AnyValue {
                                value: Some(Value::StringValue("GET".into())),
                            }),
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                schema_url: String::new(),
            }],
        };
        let bytes = request.encode_to_vec();
        let lines = otlp_protobuf_to_lines(&bytes).unwrap();
        let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        let attrs = v["attributes"].as_array().unwrap();
        assert_eq!(attrs.len(), 1);
        assert_eq!(attrs[0]["key"], "http.method");
        assert_eq!(attrs[0]["value"]["stringValue"], "GET");
    }

    #[test]
    fn test_otlp_protobuf_to_lines_recognized_by_otlp_parser() {
        use crate::parser::LogFormatParser;
        use crate::parser::otlp::OtlpParser;

        let bytes = make_protobuf_request("svc", 1_000_000_000, 9, "test message");
        let lines = otlp_protobuf_to_lines(&bytes).unwrap();
        let parser = OtlpParser;
        assert!(
            parser.parse_line(lines[0].as_bytes()).is_some(),
            "OtlpParser must recognize the output"
        );
    }

    #[tokio::test]
    async fn test_spawn_otlp_http_receiver_protobuf_accepted() {
        use axum::body::Body;
        use http::Request;
        use tower::ServiceExt;

        let bytes = make_protobuf_request("proto-svc", 1_000_000_000, 9, "proto body");
        let (app, _tx, rx, tmp) = otlp_test_router();
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/logs")
                    .header("content-type", "application/x-protobuf")
                    .body(Body::from(bytes))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert!(rx.has_changed().unwrap(), "watch should have fired");
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(content.contains("proto body"));
        assert!(content.contains("proto-svc"));
    }

    #[tokio::test]
    async fn test_spawn_otlp_http_receiver_protobuf_invalid_returns_bad_request() {
        use axum::body::Body;
        use http::Request;
        use tower::ServiceExt;

        let (app, _tx, _rx, _tmp) = otlp_test_router();
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/logs")
                    .header("content-type", "application/x-protobuf")
                    .body(Body::from(b"not valid protobuf garbage!!!".as_ref()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_spawn_otlp_http_receiver_protobuf_gzip_accepted() {
        use axum::body::Body;
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use http::Request;
        use std::io::Write as _;
        use tower::ServiceExt;

        let bytes = make_protobuf_request("gzip-proto-svc", 1_000_000_000, 9, "gzip proto body");
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&bytes).unwrap();
        let compressed = encoder.finish().unwrap();

        let (app, _tx, rx, tmp) = otlp_test_router();
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/logs")
                    .header("content-type", "application/x-protobuf")
                    .header("content-encoding", "gzip")
                    .body(Body::from(compressed))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert!(rx.has_changed().unwrap(), "watch should have fired");
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(content.contains("gzip proto body"));
        assert!(content.contains("gzip-proto-svc"));
    }

    #[tokio::test]
    async fn test_spawn_otlp_http_receiver_gzip_json_accepted() {
        use axum::body::Body;
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use http::Request;
        use std::io::Write as _;
        use tower::ServiceExt;

        let payload = otlp_payload("gzip-svc", &[("1000000000", 9, "gzip hello")]);
        let json_body = serde_json::to_string(&payload).unwrap();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(json_body.as_bytes()).unwrap();
        let compressed = encoder.finish().unwrap();

        let (app, _tx, rx, tmp) = otlp_test_router();
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/logs")
                    .header("content-type", "application/json")
                    .header("content-encoding", "gzip")
                    .body(Body::from(compressed))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert!(rx.has_changed().unwrap(), "watch should have fired");
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(
            content.contains("gzip hello"),
            "decompressed content expected"
        );
        assert!(content.contains("gzip-svc"));
    }

    #[test]
    fn test_otlp_export_request_to_lines_basic() {
        use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
        use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue, any_value::Value};
        use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
        use opentelemetry_proto::tonic::resource::v1::Resource;

        let request = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: Some(Resource {
                    attributes: vec![KeyValue {
                        key: "service.name".into(),
                        value: Some(AnyValue {
                            value: Some(Value::StringValue("grpc-svc".into())),
                        }),
                    }],
                    dropped_attributes_count: 0,
                }),
                scope_logs: vec![ScopeLogs {
                    log_records: vec![LogRecord {
                        time_unix_nano: 1_000_000_000,
                        severity_number: 9,
                        body: Some(AnyValue {
                            value: Some(Value::StringValue("hello grpc".into())),
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                schema_url: String::new(),
            }],
        };
        let lines = otlp_export_request_to_lines(&request);
        assert_eq!(lines.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(v["service.name"], "grpc-svc");
        assert_eq!(v["timeUnixNano"], "1000000000");
        assert_eq!(v["body"]["stringValue"], "hello grpc");
    }

    #[tokio::test]
    async fn test_spawn_otlp_grpc_receiver_accepts_logs() {
        use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
        use opentelemetry_proto::tonic::collector::logs::v1::logs_service_client::LogsServiceClient;
        use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue, any_value::Value};
        use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
        use opentelemetry_proto::tonic::resource::v1::Resource;

        let port = portpicker();
        let (mut rx, tmp) = spawn_otlp_grpc_receiver(port).await.unwrap();

        let endpoint = format!("http://127.0.0.1:{port}");
        let mut client = LogsServiceClient::connect(endpoint).await.unwrap();

        let request = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: Some(Resource {
                    attributes: vec![KeyValue {
                        key: "service.name".into(),
                        value: Some(AnyValue {
                            value: Some(Value::StringValue("grpc-test-svc".into())),
                        }),
                    }],
                    dropped_attributes_count: 0,
                }),
                scope_logs: vec![ScopeLogs {
                    log_records: vec![LogRecord {
                        time_unix_nano: 1_000_000_000,
                        severity_number: 9,
                        body: Some(AnyValue {
                            value: Some(Value::StringValue("hello from grpc".into())),
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                schema_url: String::new(),
            }],
        };
        client.export(request).await.unwrap();

        rx.changed().await.unwrap();
        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(content.contains("hello from grpc"));
        assert!(content.contains("grpc-test-svc"));
    }

    #[tokio::test]
    async fn test_spawn_otlp_grpc_receiver_port_conflict() {
        let port = portpicker();
        let _listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
            .await
            .unwrap();
        let result = spawn_otlp_grpc_receiver(port).await;
        assert!(result.is_err());
    }

    fn portpicker() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }
}
