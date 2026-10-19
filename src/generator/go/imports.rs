use std::collections::{BTreeMap, BTreeSet};

use crate::error::{Error, Result};
use crate::generator::names::NameScope;

/// Prefer the backend's conventional imports over authored type preferences and
/// protobuf package names. Opaque authored bindings take precedence over all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ImportKind {
    Generated,
    Authored,
    Proto,
    /// An authored import path may only be referenced by an opaque helper.
    Fallback,
}

#[derive(Debug, Default)]
struct Import {
    preferences: BTreeSet<(ImportKind, String)>,
    fixed: BTreeSet<String>,
    alias: Option<String>,
}

/// A file's imports are discovered before assigning aliases. Renderers request
/// references through the same registry on both discovery and final emission.
/// No already-rendered expression is rewritten when an alias changes.
#[derive(Debug, Default)]
pub(super) struct GoImports {
    imports: BTreeMap<String, Import>,
    names: NameScope,
    opaque_qualifiers: BTreeSet<String>,
    frozen: bool,
}

impl GoImports {
    pub(super) fn reserve(&mut self, name: &str) {
        if !self.frozen {
            self.names.reserve(name);
        }
    }

    pub(super) fn require(&mut self, path: &str, preferred: &str, kind: ImportKind) -> String {
        if self.frozen {
            return self.imports[path]
                .alias
                .as_ref()
                .expect("every discovered import has an alias")
                .clone();
        }
        self.imports
            .entry(path.to_string())
            .or_default()
            .preferences
            .insert((kind, preferred.to_string()));
        preferred.to_string()
    }

    /// An opaque authored expression cannot be safely requalified. Its import
    /// must retain the authored binding, or generation must fail.
    pub(super) fn require_fixed(&mut self, path: &str, alias: &str) {
        if !self.frozen {
            self.imports
                .entry(path.to_string())
                .or_default()
                .fixed
                .insert(alias.to_string());
        }
    }

    pub(super) fn preserve_expression(&mut self, expr: &str) {
        if !self.frozen {
            self.opaque_qualifiers.extend(referenced_qualifiers(expr));
        }
    }

    pub(super) fn freeze(&mut self) -> Result<()> {
        let conflict = |reason| Error::UnsupportedGoType {
            context: "Go import bindings".to_string(),
            reason,
        };
        for alias in &self.opaque_qualifiers {
            let candidates: Vec<_> = self
                .imports
                .iter_mut()
                .filter(|(_, import)| {
                    import
                        .preferences
                        .iter()
                        .any(|(_, preferred)| preferred == alias)
                        || import.fixed.contains(alias)
                })
                .collect();
            if candidates.len() > 1 {
                return Err(conflict(format!(
                    "opaque expression qualifier `{alias}` refers to multiple imports; use a distinct qualifier in the authored references"
                )));
            }
            if let Some((_, import)) = candidates.into_iter().next() {
                import.fixed.insert(alias.clone());
            }
        }
        for (path, import) in &mut self.imports {
            if import.fixed.len() > 1 {
                return Err(conflict(format!(
                    "opaque expressions require conflicting aliases {:?} for `{path}`; use one qualifier",
                    import.fixed
                )));
            }
            if let Some(alias) = import.fixed.first() {
                if alias == "_" || super::is_go_keyword(alias) {
                    return Err(conflict(format!(
                        "fixed import qualifier `{alias}` for `{path}` is not usable; change the authored qualifier"
                    )));
                }
                if self.names.contains(alias) {
                    return Err(conflict(format!(
                        "authored import qualifier `{alias}` for `{path}` conflicts with another binding; change the authored qualifier"
                    )));
                }
                self.names.reserve(alias);
                import.alias = Some(alias.clone());
            }
        }
        let mut pending: Vec<_> = self
            .imports
            .iter_mut()
            .filter(|(_, import)| import.alias.is_none())
            .collect();
        pending.sort_by(|(path_a, a), (path_b, b)| {
            a.preferences
                .first()
                .cmp(&b.preferences.first())
                .then(path_a.cmp(path_b))
        });
        for (_, import) in pending {
            let (_, preferred) = import
                .preferences
                .first()
                .expect("import has a preferred alias");
            let preferred = if super::is_go_keyword(preferred) {
                format!("{preferred}_")
            } else {
                preferred.clone()
            };
            import.alias = Some(self.names.fresh(&preferred));
        }
        self.frozen = true;
        Ok(())
    }

    pub(super) fn paths(&self) -> Vec<(String, String)> {
        self.imports
            .iter()
            .map(|(path, import)| {
                let alias = import.alias.clone().unwrap_or_else(|| {
                    import.fixed.first().cloned().unwrap_or_else(|| {
                        import
                            .preferences
                            .first()
                            .expect("import has a preference")
                            .1
                            .clone()
                    })
                });
                (path.clone(), alias)
            })
            .collect()
    }
}

/// Only code identifiers count as uses: documentation and string/rune literals
/// must not keep an otherwise unused import alive.
pub(super) fn referenced_qualifiers(code: &str) -> BTreeSet<String> {
    let bytes = code.as_bytes();
    let mut index = 0;
    let mut qualifiers = BTreeSet::new();
    while index < bytes.len() {
        match bytes[index] {
            b'"' | b'\'' | b'`' => {
                let quote = bytes[index];
                index += 1;
                while index < bytes.len() {
                    let byte = bytes[index];
                    index += 1;
                    if byte == quote {
                        break;
                    }
                    if byte == b'\\' && quote != b'`' {
                        index = (index + 1).min(bytes.len());
                    }
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index < bytes.len() {
                    if bytes.get(index..index + 2) == Some(b"*/") {
                        index += 2;
                        break;
                    }
                    index += 1;
                }
            }
            byte if super::is_go_ident_start(byte) => {
                let start = index;
                index += 1;
                while index < bytes.len() && super::is_go_ident_continue(bytes[index]) {
                    index += 1;
                }
                if super::next_non_whitespace(bytes, index) == Some(b'.')
                    && super::previous_non_whitespace(bytes, start) != Some(b'.')
                {
                    qualifiers.insert(code[start..index].to_string());
                }
            }
            _ => index += 1,
        }
    }
    qualifiers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_are_independent_of_discovery_order() {
        let allocate = |reverse: bool| {
            let mut imports = GoImports::default();
            let mut requests = [
                ("example.org/proto/workflow", "workflow", ImportKind::Proto),
                (
                    "go.temporal.io/sdk/workflow",
                    "workflow",
                    ImportKind::Generated,
                ),
                (
                    "example.org/proto/workflow2",
                    "workflow2",
                    ImportKind::Proto,
                ),
            ];
            if reverse {
                requests.reverse();
            }
            for (path, alias, kind) in requests {
                imports.require(path, alias, kind);
            }
            imports.reserve("workflow2");
            imports.freeze().unwrap();
            imports.paths()
        };
        assert_eq!(allocate(false), allocate(true));
        assert_eq!(allocate(false)[1].1, "workflow22");
        assert_eq!(allocate(false)[2].1, "workflow");
    }

    #[test]
    fn fixed_bindings_take_priority_and_conflicts_are_errors() {
        let mut imports = GoImports::default();
        imports.require(
            "go.temporal.io/sdk/workflow",
            "workflow",
            ImportKind::Generated,
        );
        imports.require_fixed("example.org/helper", "workflow");
        imports.freeze().unwrap();
        assert_eq!(
            imports.require(
                "go.temporal.io/sdk/workflow",
                "workflow",
                ImportKind::Generated
            ),
            "workflow2"
        );
        let mut imports = GoImports::default();
        imports.require_fixed("example.org/first", "helper");
        imports.require_fixed("example.org/second", "helper");
        assert!(imports.freeze().is_err());
    }

    #[test]
    fn opaque_expressions_pin_bindings_but_not_mentions_in_strings() {
        let mut imports = GoImports::default();
        imports.require("example.org/types", "workflow", ImportKind::Authored);
        imports.preserve_expression(r#"workflow.Convert(request.Name, "workflow2.Value")"#);
        imports.require(
            "go.temporal.io/sdk/workflow",
            "workflow",
            ImportKind::Generated,
        );
        assert!(imports.freeze().is_err());
        assert_eq!(
            referenced_qualifiers(
                "// unused.Call\n/* other.Call */ actual.Call(\"fake.Call\", `raw.Call`)"
            ),
            BTreeSet::from(["actual".to_string()])
        );
    }
}
