/// A preset choice for the search bar's time-range dropdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeRangePreset {
    Last15Min,
    LastHour,
    Last24Hours,
    Today,
    AllTime,
}

/// Label shown on the dropdown's closed button and its own menu entry.
pub fn preset_label(preset: TimeRangePreset) -> &'static str {
    match preset {
        TimeRangePreset::Last15Min => "Last 15 min",
        TimeRangePreset::LastHour => "Last 1 hour",
        TimeRangePreset::Last24Hours => "Last 24 hours",
        TimeRangePreset::Today => "Today",
        TimeRangePreset::AllTime => "All time",
    }
}

/// The `:date-filter` expression a preset translates to, in the exact
/// grammar `crate::filters::parse_date_filter` accepts (`"> YYYY-MM-DD
/// HH:MM:SS"`, a `ComparisonMode::FullDatetime` lower bound). `AllTime`
/// has no expression — selecting it means "clear the date filter", not
/// "filter from the Unix epoch".
pub fn date_filter_expression(
    preset: TimeRangePreset,
    now: time::OffsetDateTime,
) -> Option<String> {
    let since = match preset {
        TimeRangePreset::Last15Min => now - time::Duration::minutes(15),
        TimeRangePreset::LastHour => now - time::Duration::hours(1),
        TimeRangePreset::Last24Hours => now - time::Duration::hours(24),
        TimeRangePreset::Today => now.replace_time(time::Time::MIDNIGHT),
        TimeRangePreset::AllTime => return None,
    };
    Some(format!(
        "> {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        since.year(),
        since.month() as u8,
        since.day(),
        since.hour(),
        since.minute(),
        since.second()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> time::OffsetDateTime {
        time::Date::from_calendar_date(year, time::Month::try_from(month).unwrap(), day)
            .unwrap()
            .with_hms(hour, minute, second)
            .unwrap()
            .assume_utc()
    }

    #[test]
    fn all_time_has_no_expression() {
        let now = dt(2024, 6, 15, 12, 30, 0);
        assert_eq!(date_filter_expression(TimeRangePreset::AllTime, now), None);
    }

    #[test]
    fn last_15_min_subtracts_15_minutes() {
        let now = dt(2024, 6, 15, 12, 30, 0);
        assert_eq!(
            date_filter_expression(TimeRangePreset::Last15Min, now),
            Some("> 2024-06-15 12:15:00".to_string())
        );
    }

    #[test]
    fn last_hour_subtracts_an_hour() {
        let now = dt(2024, 6, 15, 12, 30, 0);
        assert_eq!(
            date_filter_expression(TimeRangePreset::LastHour, now),
            Some("> 2024-06-15 11:30:00".to_string())
        );
    }

    #[test]
    fn last_24_hours_crosses_a_day_boundary() {
        let now = dt(2024, 6, 15, 1, 0, 0);
        assert_eq!(
            date_filter_expression(TimeRangePreset::Last24Hours, now),
            Some("> 2024-06-14 01:00:00".to_string())
        );
    }

    #[test]
    fn today_is_midnight_of_the_current_day() {
        let now = dt(2024, 6, 15, 18, 45, 30);
        assert_eq!(
            date_filter_expression(TimeRangePreset::Today, now),
            Some("> 2024-06-15 00:00:00".to_string())
        );
    }

    #[test]
    fn every_non_all_time_preset_round_trips_through_the_real_parser() {
        let now = dt(2024, 6, 15, 12, 30, 0);
        for preset in [
            TimeRangePreset::Last15Min,
            TimeRangePreset::LastHour,
            TimeRangePreset::Last24Hours,
            TimeRangePreset::Today,
        ] {
            let expr = date_filter_expression(preset, now).unwrap();
            crate::filters::parse_date_filter(&expr).unwrap_or_else(|e| {
                panic!("{preset:?} produced an unparseable expression {expr:?}: {e}")
            });
        }
    }

    #[test]
    fn labels_match_the_mockup() {
        assert_eq!(preset_label(TimeRangePreset::LastHour), "Last 1 hour");
        assert_eq!(preset_label(TimeRangePreset::AllTime), "All time");
    }
}
