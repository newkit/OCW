//! A document's selection: the things picked in the views (a road edge, a
//! node, a via-point). Not part of the edit history: selecting is no
//! change of the document. After every change the selection drops what
//! the document no longer has ([`crate::DocumentKind::contains`]).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection<I: Ord> {
    items: BTreeSet<I>,
}

impl<I: Ord> Default for Selection<I> {
    fn default() -> Self {
        Self {
            items: BTreeSet::new(),
        }
    }
}

impl<I: Ord + Clone> Selection<I> {
    /// Selects `item` alone (a plain click).
    pub fn select_only(&mut self, item: I) {
        self.items.clear();
        self.items.insert(item);
    }

    /// Adds `item` (Shift + click).
    pub fn add(&mut self, item: I) {
        self.items.insert(item);
    }

    /// Adds or removes `item` (Ctrl + click); whether it is selected now.
    pub fn toggle(&mut self, item: I) -> bool {
        if self.items.remove(&item) {
            false
        } else {
            self.items.insert(item);
            true
        }
    }

    pub fn remove(&mut self, item: &I) {
        self.items.remove(item);
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn contains(&self, item: &I) -> bool {
        self.items.contains(item)
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &I> {
        self.items.iter()
    }

    /// The only selected item, if exactly one is selected.
    pub fn single(&self) -> Option<&I> {
        (self.items.len() == 1)
            .then(|| self.items.first())
            .flatten()
    }

    /// Keeps only the items `keep` accepts.
    pub fn retain(&mut self, keep: impl Fn(&I) -> bool) {
        self.items.retain(|i| keep(i));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_shift_click_and_ctrl_click() {
        let mut s = Selection::default();
        s.select_only(3);
        assert_eq!(s.single(), Some(&3));
        s.add(5);
        assert_eq!(s.len(), 2);
        assert_eq!(s.single(), None);
        assert!(!s.toggle(3));
        assert!(s.toggle(7));
        assert_eq!(s.iter().copied().collect::<Vec<_>>(), vec![5, 7]);
        s.retain(|i| *i > 5);
        assert_eq!(s.single(), Some(&7));
        s.remove(&7);
        assert!(s.is_empty());
        s.add(1);
        s.clear();
        assert!(!s.contains(&1));
    }
}
