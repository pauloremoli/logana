#[cfg(test)]
mod tests {

    #[test]
    fn test_set_color_config() {
        let mut analyzer = crate::analyzer::LogAnalyzer::new();
        analyzer.set_color_config("error", "red", "black");
        let config = analyzer.get_color_config("error").unwrap();
        assert_eq!(config.fg, ratatui::style::Color::Red);
        assert_eq!(config.bg, ratatui::style::Color::Black);
    }

    #[test]
    fn test_set_invalid_color_config() {
        let mut analyzer = crate::analyzer::LogAnalyzer::new();
        analyzer.set_color_config("error", "invalid_color", "black");
        let config = analyzer.get_color_config("error");
        assert!(config.is_none());
    }
}
