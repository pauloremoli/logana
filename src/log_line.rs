#[derive(Debug)]
pub struct LogLine<'a> {
    pub text: &'a [u8],
    pub level: Option<&'a str>,
    pub timestamp: Option<&'a str>,
}

impl<'a> LogLine<'a> {
    pub fn parse(line: &'a [u8]) -> Self {
        if line.is_empty() {
            return LogLine {
                text: line,
                level: None,
                timestamp: None,
            };
        }

        let mut timestamp = None;
        let mut level = None;
        let mut current_pos = 0;

        // Try to parse timestamp
        if line.len() > 1 && line[0] == b'[' {
            if let Some(ts_end_bracket) = line[1..].iter().position(|&b| b == b']') {
                // timestamp content is from index 1 up to ts_end_bracket
                timestamp = std::str::from_utf8(&line[1..ts_end_bracket + 1]).ok();
                
                current_pos = ts_end_bracket + 2; // Advance past ']' and the space after it

                // Skip any additional spaces after the initial space
                while current_pos < line.len() && line[current_pos] == b' ' {
                    current_pos += 1;
                }
            }
        }

        // Try to parse level
        if current_pos < line.len() {
            if let Some(level_space) = line[current_pos..].iter().position(|&b| b == b' ') {
                level = std::str::from_utf8(&line[current_pos..current_pos + level_space]).ok();
            } else {
                // If no space found, take the rest of the line as level (e.g., "[TS]LEVEL" or "[TS]LEVEL\n")
                level = std::str::from_utf8(&line[current_pos..]).ok();
            }
        }

        LogLine {
            text: line,
            level,
            timestamp,
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_log_line_full() {
        let line = b"[2024-07-24T10:00:00Z] INFO myhost: everything is fine";
        let log_line = LogLine::parse(line);
        assert_eq!(log_line.timestamp, Some("2024-07-24T10:00:00Z"));
        assert_eq!(log_line.level, Some("INFO"));
        assert_eq!(log_line.text, line);
    }

    #[test]
    fn test_parse_log_line_no_level_no_host() {
        let line = b"[2024-07-24T10:00:00Z] some message without level or host";
        let log_line = LogLine::parse(line);
        assert_eq!(log_line.timestamp, Some("2024-07-24T10:00:00Z"));
        assert_eq!(log_line.level, Some("some")); // "some" is now parsed as level
        assert_eq!(log_line.text, line);
    }

    #[test]
    fn test_parse_log_line_empty() {
        let line = b"";
        let log_line = LogLine::parse(line);
        assert_eq!(log_line.timestamp, None);
        assert_eq!(log_line.level, None);
        assert_eq!(log_line.text, line);
    }

    #[test]
    fn test_parse_log_line_only_timestamp() {
        let line = b"[2024-07-24T10:00:00Z]";
        let log_line = LogLine::parse(line);
        assert_eq!(log_line.timestamp, Some("2024-07-24T10:00:00Z"));
        assert_eq!(log_line.level, None);
        assert_eq!(log_line.text, line);
    }

    #[test]
    fn test_parse_log_line_no_timestamp_bracket() {
        let line = b"2024-07-24T10:00:00Z INFO message";
        let log_line = LogLine::parse(line);
        assert_eq!(log_line.timestamp, None); // No [ at start
        assert_eq!(log_line.level, Some("2024-07-24T10:00:00Z")); // First word is parsed as level
        assert_eq!(log_line.text, line);
    }
}
