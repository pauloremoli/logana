use crate::ingestion::FileReader;
use std::collections::HashSet;

#[derive(Default)]
pub struct MarkManager {
    marks: HashSet<usize>,
}

impl MarkManager {
    pub fn toggle(&mut self, line_idx: usize) {
        if self.marks.contains(&line_idx) {
            self.marks.remove(&line_idx);
        } else {
            self.marks.insert(line_idx);
        }
    }

    pub fn is_marked(&self, line_idx: usize) -> bool {
        self.marks.contains(&line_idx)
    }

    pub fn get_indices(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self.marks.iter().copied().collect();
        v.sort_unstable();
        v
    }

    pub fn set(&mut self, indices: Vec<usize>) {
        self.marks = indices.into_iter().collect();
    }

    pub fn get_lines<'a>(&self, reader: &'a FileReader) -> Vec<&'a [u8]> {
        let mut indices: Vec<usize> = self.marks.iter().copied().collect();
        indices.sort_unstable();
        indices
            .into_iter()
            .filter(|&i| i < reader.line_count())
            .map(|i| reader.get_line(i))
            .collect()
    }

    pub fn clear(&mut self) {
        self.marks.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingestion::FileReader;

    #[test]
    fn test_toggle_and_query() {
        let mut m = MarkManager::default();
        assert!(!m.is_marked(0));
        assert!(!m.is_marked(5));

        m.toggle(0);
        m.toggle(5);
        assert!(m.is_marked(0));
        assert!(m.is_marked(5));

        m.toggle(0);
        assert!(!m.is_marked(0));
        assert!(m.is_marked(5));

        assert_eq!(m.get_indices(), vec![5]);
    }

    #[test]
    fn test_set_replaces_all() {
        let mut m = MarkManager::default();
        m.set(vec![1, 3, 7]);
        assert!(m.is_marked(1));
        assert!(m.is_marked(3));
        assert!(m.is_marked(7));
        assert!(!m.is_marked(0));
    }

    #[test]
    fn test_clear() {
        let mut m = MarkManager::default();
        m.toggle(0);
        m.toggle(2);
        m.clear();
        assert!(m.get_indices().is_empty());
    }

    #[test]
    fn test_get_lines() {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, "line zero").unwrap();
        writeln!(f, "line one").unwrap();
        writeln!(f, "line two").unwrap();
        let reader = FileReader::new(f.path().to_str().unwrap()).unwrap();

        let mut m = MarkManager::default();
        m.toggle(0);
        m.toggle(2);

        let lines = m.get_lines(&reader);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], b"line zero");
        assert_eq!(lines[1], b"line two");
    }
}
