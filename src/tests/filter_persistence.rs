#[cfg(test)]
mod tests {

    use crate::analyzer::{FilterType, LogAnalyzer};
    use tempfile::NamedTempFile;

    #[test]
    fn test_save_and_load_filters() -> anyhow::Result<()> {
        let mut analyzer = LogAnalyzer::new();
        analyzer.add_filter("error".to_string(), FilterType::Include);
        analyzer.add_filter("info".to_string(), FilterType::Exclude);

        let file = NamedTempFile::new()?;
        let path = file.path().to_str().unwrap();

        analyzer.save_filters(path)?;

        let mut new_analyzer = LogAnalyzer::new();
        new_analyzer.load_filters(path)?;

        assert_eq!(new_analyzer.filters.len(), 2);
        assert_eq!(new_analyzer.filters[0].pattern, "error");
        assert_eq!(new_analyzer.filters[0].filter_type, FilterType::Include);
        assert_eq!(new_analyzer.filters[1].pattern, "info");
        assert_eq!(new_analyzer.filters[1].filter_type, FilterType::Exclude);

        Ok(())
    }
}
