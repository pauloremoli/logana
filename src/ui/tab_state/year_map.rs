use rayon::iter::{IntoParallelIterator, ParallelIterator};

use crate::filters::bsd_month_from_timestamp;
use crate::ingestion::FileReader;
use crate::parser::LogFormatParser;

/// Maps log-file line indices to the correct calendar year for BSD-format
/// timestamps (e.g. syslog `"Feb 22 10:15:30"`).
///
/// Built by a single sequential scan of the file.  When the month number
/// decreases significantly (Dec→Jan, difference > 3) we infer a year boundary
/// and increment the year counter.  All subsequent lines belong to the new year.
///
/// Year lookups are O(log n) via binary search on the transitions list.
#[derive(Debug)]
pub struct YearMap {
    /// Sorted `(first_line_index, year)` pairs.  The year applies to all lines
    /// from `first_line_index` up to (but not including) the next transition.
    transitions: Vec<(usize, i32)>,
}

impl YearMap {
    pub fn build<P: LogFormatParser + ?Sized>(
        file_reader: &FileReader,
        parser: &P,
        start_year: i32,
    ) -> Self {
        let count = file_reader.line_count();

        // Phase 1: parallel extraction
        let months: Vec<Option<u32>> = (0..count)
            .into_par_iter()
            .map(|i| {
                let line = file_reader.get_line(i);
                parser
                    .parse_timestamp(line)
                    .and_then(bsd_month_from_timestamp)
            })
            .collect();

        // Phase 2: sequential transitions
        let mut transitions = Vec::with_capacity(16);
        transitions.push((0, start_year));

        let mut current_year = start_year;
        let mut prev_month = 0;

        for (i, month_opt) in months.iter().enumerate() {
            if let Some(month) = month_opt {
                let month = *month;

                if prev_month > 0 && month < prev_month && prev_month - month > 3 {
                    current_year += 1;
                    transitions.push((i, current_year));
                }

                prev_month = month;
            }
        }

        YearMap { transitions }
    }

    /// Return the calendar year for the given line index (O(log n)).
    pub fn year_for_line(&self, line_idx: usize) -> i32 {
        match self
            .transitions
            .binary_search_by_key(&line_idx, |&(idx, _)| idx)
        {
            Ok(pos) => self.transitions[pos].1,
            Err(0) => self.transitions[0].1,
            Err(pos) => self.transitions[pos - 1].1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_from(transitions: Vec<(usize, i32)>) -> YearMap {
        YearMap { transitions }
    }

    #[test]
    fn test_year_for_line_single_year() {
        let m = map_from(vec![(0, 2024)]);
        assert_eq!(m.year_for_line(0), 2024);
        assert_eq!(m.year_for_line(100), 2024);
        assert_eq!(m.year_for_line(usize::MAX), 2024);
    }

    #[test]
    fn test_year_for_line_transition() {
        let m = map_from(vec![(0, 2023), (50, 2024)]);
        assert_eq!(m.year_for_line(0), 2023);
        assert_eq!(m.year_for_line(49), 2023);
        assert_eq!(m.year_for_line(50), 2024);
        assert_eq!(m.year_for_line(200), 2024);
    }

    #[test]
    fn test_year_for_line_exact_transition_boundary() {
        let m = map_from(vec![(0, 2022), (10, 2023), (20, 2024)]);
        assert_eq!(m.year_for_line(9), 2022);
        assert_eq!(m.year_for_line(10), 2023);
        assert_eq!(m.year_for_line(19), 2023);
        assert_eq!(m.year_for_line(20), 2024);
    }

    #[test]
    fn test_build_all_same_month_no_transitions() {
        let data = b"Dec  1 10:00:00 host sshd: msg\nDec  2 10:00:00 host sshd: msg\nDec  3 10:00:00 host sshd: msg\n";
        let reader = crate::ingestion::FileReader::from_bytes(data.to_vec());
        let parser = crate::parser::syslog::SyslogParser::default();
        let map = YearMap::build(&reader, &parser, 2024);
        assert_eq!(map.year_for_line(0), 2024);
        assert_eq!(map.year_for_line(2), 2024);
    }

    #[test]
    fn test_build_detects_dec_to_jan_boundary() {
        let data = b"Dec 31 23:59:59 host sshd: msg\nJan  1 00:00:01 host sshd: msg\n";
        let reader = crate::ingestion::FileReader::from_bytes(data.to_vec());
        let parser = crate::parser::syslog::SyslogParser::default();
        let map = YearMap::build(&reader, &parser, 2024);
        assert_eq!(map.year_for_line(0), 2024);
        assert_eq!(map.year_for_line(1), 2025);
    }

    #[test]
    fn test_build_empty_file() {
        let reader = crate::ingestion::FileReader::from_bytes(b"".to_vec());
        let parser = crate::parser::syslog::SyslogParser::default();
        let map = YearMap::build(&reader, &parser, 2024);
        assert_eq!(map.year_for_line(0), 2024);
    }

    #[test]
    fn test_build_non_parseable_lines_skipped() {
        let data =
            b"Dec 31 23:59:00 host sshd: msg\nnot-a-syslog-line\nJan  1 00:00:01 host sshd: msg\n";
        let reader = crate::ingestion::FileReader::from_bytes(data.to_vec());
        let parser = crate::parser::syslog::SyslogParser::default();
        let map = YearMap::build(&reader, &parser, 2023);
        assert_eq!(map.year_for_line(0), 2023);
        assert_eq!(map.year_for_line(2), 2024);
    }

    #[test]
    fn test_build_multiple_year_transitions() {
        let data = b"Dec 31 00:00:00 host a: m\nJan  1 00:00:00 host a: m\nDec 31 00:00:00 host a: m\nJan  1 00:00:00 host a: m\n";
        let reader = crate::ingestion::FileReader::from_bytes(data.to_vec());
        let parser = crate::parser::syslog::SyslogParser::default();
        let map = YearMap::build(&reader, &parser, 2022);
        assert_eq!(map.year_for_line(0), 2022);
        assert_eq!(map.year_for_line(1), 2023);
        assert_eq!(map.year_for_line(2), 2023);
        assert_eq!(map.year_for_line(3), 2024);
    }
}
