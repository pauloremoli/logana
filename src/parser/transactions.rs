//! Multiline transactions with operation blocks and voting/execution/abort stages.
use std::sync::LazyLock;

use regex::Regex;

use super::{ContinuationWalkResult, DisplayParts, GroupItem, LogFormatParser, push_extra_field};

static HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^### Transaction (\d+) started at: ",
        r"(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d+)$"
    ))
    .unwrap()
});

#[derive(Debug)]
pub struct TransactionsParser;

fn item<'a>(key: &'a str, value: &'a str) -> GroupItem<'a> {
    let mut item = GroupItem::default();
    push_extra_field(&mut item.fields, key, value);
    item
}

impl LogFormatParser for TransactionsParser {
    fn name(&self) -> &str {
        "transactions"
    }

    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>> {
        let text = std::str::from_utf8(line)
            .ok()?
            .trim_end_matches(['\r', '\n']);
        let captures = HEADER.captures(text)?;
        let timestamp = captures.get(2)?.as_str();
        let mut extra_fields = Vec::new();
        push_extra_field(&mut extra_fields, "id", captures.get(1)?.as_str());
        push_extra_field(&mut extra_fields, "started_at", timestamp);
        Some(DisplayParts {
            timestamp: Some(timestamp),
            message: Some(text),
            extra_fields,
            ..Default::default()
        })
    }

    fn wants_continuation_walk(&self) -> bool {
        true
    }

    // Handle the terminator inside the walk so its timestamp is extracted too.
    // is_continuation_end would exclude that line before the walk sees it.
    fn walk_continuation<'a>(&self, lines: &[&'a [u8]]) -> ContinuationWalkResult<'a> {
        let mut result = ContinuationWalkResult::default();
        let mut operations: Vec<GroupItem<'a>> = Vec::new();
        let mut rounds = Vec::new();
        let mut votes = Vec::new();
        let mut executions = Vec::new();
        let mut aborts = Vec::new();
        let mut operation_open = false;
        let mut round = None;
        for &line in lines {
            let Ok(text) = std::str::from_utf8(line) else {
                continue;
            };
            let text = text.trim_end_matches(['\r', '\n']);
            if let Some(value) = text.strip_prefix("### Transaction ended at: ") {
                push_extra_field(&mut result.flat_fields, "ended_at", value);
                break;
            }
            if self.parse_line(line).is_some() {
                break;
            }
            if let Some(value) = text.strip_prefix("Status: ") {
                push_extra_field(&mut result.flat_fields, "status", value);
            } else if let Some(value) = text.strip_prefix("User: ") {
                push_extra_field(&mut result.flat_fields, "user", value);
            } else if let Some(value) = text.strip_prefix("operation_type: ") {
                operations.push(item("operation_type", value));
                operation_open = true;
            } else if let Some(value) = text
                .strip_prefix("Voting Round ")
                .and_then(|s| s.strip_suffix(':'))
            {
                operation_open = false;
                round = Some(value);
                rounds.push(item("round", value));
            } else if matches!(text, "Execute stage:" | "Abort stage:") {
                operation_open = false;
            } else if let Some(rest) = text.strip_prefix("ReqId: ") {
                operation_open = false;
                let Some((req_id, rest)) = rest.split_once(", Actor: ") else {
                    continue;
                };
                let Some((actor, status)) = rest.split_once(", ") else {
                    continue;
                };
                if req_id.is_empty()
                    || !req_id.bytes().all(|b| b.is_ascii_digit())
                    || actor.is_empty()
                {
                    continue;
                }
                let (items, key, value) = if let Some(value) = status.strip_prefix("VotingStatus: ")
                {
                    (&mut votes, "voting_status", value)
                } else if let Some(value) = status.strip_prefix("ExecuteStatus: ") {
                    (&mut executions, "execute_status", value)
                } else if let Some(value) = status.strip_prefix("AbortStatus: ") {
                    (&mut aborts, "abort_status", value)
                } else {
                    continue;
                };
                let mut entry = item("req_id", req_id);
                push_extra_field(&mut entry.fields, "actor", actor);
                push_extra_field(&mut entry.fields, key, value);
                if key == "voting_status"
                    && let Some(round) = round
                {
                    push_extra_field(&mut entry.fields, "round", round);
                }
                items.push(entry);
            } else if operation_open && text.starts_with(char::is_whitespace) {
                // Protobuf text attributes have varying keys. Keep repeated keys
                // and strip only their surrounding quotes, preserving raw values.
                if let Some((key, value)) = text.trim().split_once(':') {
                    let value = value.trim();
                    if !key.is_empty() && !value.is_empty() {
                        let value = value
                            .strip_prefix('"')
                            .and_then(|s| s.strip_suffix('"'))
                            .unwrap_or(value);
                        push_extra_field(&mut operations.last_mut().unwrap().fields, key, value);
                    }
                }
            }
        }
        result.groups = vec![
            ("operations", operations),
            ("voting_rounds", rounds),
            ("votes", votes),
            ("executions", executions),
            ("abort_stage", aborts),
        ];
        result
    }

    fn collect_field_names(&self, lines: &[&[u8]]) -> Vec<String> {
        let mut names: Vec<String> = [
            "id",
            "timestamp",
            "started_at",
            "status",
            "user",
            "ended_at",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        for body in lines.split(|line| self.parse_line(line).is_some()) {
            for (group, items) in self.walk_continuation(body).groups {
                for item in items {
                    for (_, key, _) in item.fields {
                        let name = format!("{group}.{key}");
                        if !names.contains(&name) {
                            names.push(name);
                        }
                    }
                }
            }
        }
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::FieldSemantic;

    const LOG: &str = "__rotate_count 1\n### Transaction 101 started at: 2035-01-02 03:04:05.100\nStatus: COMPLETED\nUser: Casey\nOperations: \noperation_type: UPDATE\nobject {\n  object_name: \"DemoWidget0\"\n  class_name: \"DemoWidget\"\n  [example.objects.DemoWidget.object] {\n    sampleValue: 40\n  }\n}\noperation_type: CREATE\nobject {\n  object_name: \"DemoWidget1\"\n}\nVoting Round 0:\nReqId: 1, Actor: demo_worker::instance1, VotingStatus: ABSTAIN\nVoting Round 1:\nReqId: 2, Actor: demo_worker::instance1, VotingStatus: YES\nExecute stage:\nReqId: 3, Actor: demo_worker::instance1, ExecuteStatus: OK\nAbort stage:\nReqId: 4, Actor: demo_worker::instance1, AbortStatus: OK\n### Transaction ended at: 2035-01-02 03:04:05.200\nStatus: STRAY\n";

    #[test]
    fn parses_transaction_stages_and_bounds_operation_fields() {
        let parser = TransactionsParser;
        let lines: Vec<_> = LOG.lines().map(str::as_bytes).collect();
        let parts = parser.parse_line(lines[1]).unwrap();
        assert_eq!(parts.timestamp, Some("2035-01-02 03:04:05.100"));
        assert!(
            parts
                .extra_fields
                .contains(&(FieldSemantic::Extra, "id", "101"))
        );
        let result = parser.walk_continuation(&lines[2..]);
        assert!(
            result
                .flat_fields
                .contains(&(FieldSemantic::Extra, "status", "COMPLETED"))
        );
        assert!(result.flat_fields.contains(&(
            FieldSemantic::Extra,
            "ended_at",
            "2035-01-02 03:04:05.200"
        )));
        assert!(!result.flat_fields.iter().any(|(_, _, v)| *v == "STRAY"));
        let group = |name| &result.groups.iter().find(|(n, _)| *n == name).unwrap().1;
        assert_eq!(group("operations").len(), 2);
        assert!(group("operations")[0].fields.contains(&(
            FieldSemantic::Extra,
            "sampleValue",
            "40"
        )));
        assert_eq!(group("operations")[1].fields.len(), 2);
        assert_eq!(group("votes").len(), 2);
        for (index, round) in ["0", "1"].into_iter().enumerate() {
            assert!(
                group("votes")[index]
                    .fields
                    .contains(&(FieldSemantic::Extra, "round", round))
            );
        }
        assert_eq!(group("executions").len(), 1);
        assert_eq!(group("abort_stage").len(), 1);
        assert_eq!(
            crate::parser::detect_format(&lines).unwrap().name(),
            "transactions"
        );
        let names = parser.collect_field_names(&lines);
        for name in [
            "operations.sampleValue",
            "votes.round",
            "executions.execute_status",
            "abort_stage.abort_status",
        ] {
            assert!(names.contains(&name.to_string()), "{name}");
        }
    }

    #[test]
    fn headers_reject_invalid_input_and_truncated_records_still_parse() {
        for line in [
            b"### Transaction ended at: 2035-01-02 03:04:05.200".as_slice(),
            b"### Transaction x started at: invalid",
            b"\xff",
        ] {
            assert!(TransactionsParser.parse_line(line).is_none());
        }
        let lines: Vec<_> = LOG.lines().take(13).skip(2).map(str::as_bytes).collect();
        assert_eq!(
            TransactionsParser.walk_continuation(&lines).groups[0]
                .1
                .len(),
            1
        );
        let lines: Vec<_> = LOG.lines().map(str::as_bytes).collect();
        let mut repeated = lines[2..].to_vec();
        repeated.extend_from_slice(&lines[1..]);
        let names = TransactionsParser.collect_field_names(&repeated);
        assert_eq!(names.iter().filter(|n| *n == "votes.round").count(), 1);
    }
}
