use crate::db::Comment;

#[derive(Default)]
pub struct CommentManager {
    comments: Vec<Comment>,
}

impl CommentManager {
    pub fn add(&mut self, text: String, line_indices: Vec<usize>) {
        if !line_indices.is_empty() {
            self.comments.push(Comment { text, line_indices });
        }
    }

    pub fn get(&self) -> &[Comment] {
        &self.comments
    }

    pub fn has(&self, line_idx: usize) -> bool {
        self.comments
            .iter()
            .any(|a| a.line_indices.contains(&line_idx))
    }

    pub fn set(&mut self, comments: Vec<Comment>) {
        self.comments = comments;
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.comments.len() {
            self.comments.remove(index);
        }
    }

    pub fn clear(&mut self) {
        self.comments.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_get() {
        let mut c = CommentManager::default();
        c.add("first".into(), vec![0]);
        c.add("second".into(), vec![1]);
        assert_eq!(c.get().len(), 2);
        assert_eq!(c.get()[0].text, "first");
    }

    #[test]
    fn test_add_empty_line_indices_is_noop() {
        let mut c = CommentManager::default();
        c.add("orphan".into(), vec![]);
        assert!(c.get().is_empty());
    }

    #[test]
    fn test_has() {
        let mut c = CommentManager::default();
        c.add("note".into(), vec![2, 3]);
        assert!(c.has(2));
        assert!(c.has(3));
        assert!(!c.has(0));
    }

    #[test]
    fn test_remove() {
        let mut c = CommentManager::default();
        c.add("first".into(), vec![0]);
        c.add("second".into(), vec![1]);
        c.remove(0);
        assert_eq!(c.get().len(), 1);
        assert_eq!(c.get()[0].text, "second");
    }

    #[test]
    fn test_remove_out_of_bounds_is_noop() {
        let mut c = CommentManager::default();
        c.add("only".into(), vec![0]);
        c.remove(5);
        assert_eq!(c.get().len(), 1);
    }

    #[test]
    fn test_set_replaces_all() {
        let mut c = CommentManager::default();
        c.add("old".into(), vec![0]);
        c.set(vec![crate::db::Comment {
            text: "new".into(),
            line_indices: vec![1],
        }]);
        assert_eq!(c.get().len(), 1);
        assert_eq!(c.get()[0].text, "new");
    }

    #[test]
    fn test_clear() {
        let mut c = CommentManager::default();
        c.add("note".into(), vec![0]);
        c.clear();
        assert!(c.get().is_empty());
    }
}
