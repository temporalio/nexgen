use std::collections::BTreeSet;

/// Names visible to a generated lexical scope. Backends supply valid identifiers
/// and decide which language constructs introduce a scope.
#[derive(Debug, Clone, Default)]
pub(super) struct NameScope {
    occupied: BTreeSet<String>,
}

impl NameScope {
    pub(super) fn reserve(&mut self, name: impl Into<String>) {
        self.occupied.insert(name.into());
    }

    pub(super) fn contains(&self, name: &str) -> bool {
        self.occupied.contains(name)
    }

    pub(super) fn fresh(&mut self, preferred: &str) -> String {
        if self.occupied.insert(preferred.to_string()) {
            return preferred.to_string();
        }
        for suffix in 2.. {
            let candidate = format!("{preferred}{suffix}");
            if self.occupied.insert(candidate.clone()) {
                return candidate;
            }
        }
        unreachable!("identifier suffixes are unbounded")
    }

    /// Snapshot the enclosing scope before rendering a child. Child allocations
    /// cannot leak into siblings; later parent allocations are not inherited.
    pub(super) fn child_scope(&self) -> Self {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_names_skip_reserved_suffixes_deterministically() {
        let allocate = || {
            let mut scope = NameScope::default();
            scope.reserve("item2");
            [
                scope.fresh("item"),
                scope.fresh("item"),
                scope.fresh("item"),
                scope.fresh("item2"),
            ]
        };
        assert_eq!(allocate(), ["item", "item3", "item4", "item22"]);
        assert_eq!(allocate(), allocate());
    }

    #[test]
    fn children_inherit_visible_names_without_affecting_siblings() {
        let mut parent = NameScope::default();
        parent.reserve("value");
        let mut first = parent.child_scope();
        let mut second = parent.child_scope();
        assert_eq!(first.fresh("value"), "value2");
        assert_eq!(first.fresh("local"), "local");
        assert_eq!(second.fresh("value"), "value2");
        assert_eq!(parent.fresh("local"), "local");
    }
}
