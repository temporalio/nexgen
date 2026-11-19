//! Input-independent documentation templates. Frontends bind references;
//! backends supply their emitted identifiers before escaping and wrapping prose.

use std::collections::BTreeMap;
use std::ops::Deref;

use crate::language::Language;
use crate::spec::{LanguageStringSpec, SelectedTextSpec, TextSpec};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceKind {
    Type,
    TypeExpression,
    Function,
    Method,
    Field,
    OperationParameter,
    Parameter,
    VariantCase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocReference {
    pub kind: ReferenceKind,
    /// Language-neutral declaration identity, not source-format lookup syntax.
    pub identity: String,
    pub name: String,
    pub parent: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocPart {
    Text(String),
    Name(DocReference),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocTemplate {
    source: String,
    pub parts: Vec<DocPart>,
}

impl DocTemplate {
    pub fn new(source: String, parts: Vec<DocPart>) -> Self {
        Self { source, parts }
    }

    pub fn render(&self, mut name: impl FnMut(&DocReference) -> String) -> String {
        self.parts
            .iter()
            .map(|part| match part {
                DocPart::Text(text) => text.clone(),
                DocPart::Name(reference) => name(reference),
            })
            .collect()
    }

    pub fn starts_with_name(&self) -> bool {
        self.parts
            .iter()
            .find(|part| !matches!(part, DocPart::Text(text) if text.trim().is_empty()))
            .is_some_and(|part| matches!(part, DocPart::Name(_)))
    }
}

impl From<String> for DocTemplate {
    fn from(text: String) -> Self {
        Self::new(text.clone(), vec![DocPart::Text(text)])
    }
}

impl Deref for DocTemplate {
    type Target = str;

    fn deref(&self) -> &str {
        &self.source
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LanguageDocSpec {
    pub default: Option<DocTemplate>,
    pub by_language: BTreeMap<Language, DocTemplate>,
}

impl LanguageDocSpec {
    pub fn template_for_language(&self, language: Language) -> Option<&DocTemplate> {
        self.by_language.get(&language).or(self.default.as_ref())
    }

    pub fn for_language(&self, language: Language) -> Option<&str> {
        self.template_for_language(language).map(|doc| &**doc)
    }

    pub fn is_empty(&self) -> bool {
        self.default.is_none() && self.by_language.is_empty()
    }
}

impl From<LanguageStringSpec> for LanguageDocSpec {
    fn from(text: LanguageStringSpec) -> Self {
        Self {
            default: text.default.map(Into::into),
            by_language: text
                .by_language
                .into_iter()
                .map(|(language, text)| (language, text.into()))
                .collect(),
        }
    }
}

impl TextSpec for LanguageDocSpec {
    fn is_empty(&self) -> bool {
        self.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectedDocSpec {
    pub template: Option<DocTemplate>,
    rendered: Option<String>,
    names: Vec<String>,
}

impl SelectedDocSpec {
    pub fn new(template: Option<DocTemplate>) -> Self {
        let rendered = template
            .as_ref()
            .filter(|template| {
                template
                    .parts
                    .iter()
                    .all(|part| matches!(part, DocPart::Text(_)))
            })
            .map(|template| template.render(|_| unreachable!("literal documentation")));
        Self {
            template,
            rendered,
            names: Vec::new(),
        }
    }

    pub fn value(&self) -> Option<&str> {
        assert!(
            self.template.is_none() || self.rendered.is_some(),
            "documentation references must be prepared before rendering"
        );
        self.rendered.as_deref()
    }

    pub fn is_empty(&self) -> bool {
        self.template.is_none()
    }

    pub fn starts_with_name(&self, name: &str) -> bool {
        self.template
            .as_ref()
            .is_some_and(DocTemplate::starts_with_name)
            && self.names.first().is_some_and(|first| first == name)
    }

    pub fn prepare(&mut self, mut name: impl FnMut(&DocReference) -> String) {
        self.names.clear();
        self.rendered = self.template.as_ref().map(|template| {
            template.render(|reference| {
                let name = name(reference);
                self.names.push(name.clone());
                name
            })
        });
    }

    /// Re-render references for another public surface without rewriting prose.
    pub fn map_names(&self, mut map: impl FnMut(&str) -> String) -> Option<String> {
        self.map_names_with(|_, name| map(name))
    }

    pub fn map_names_with(
        &self,
        mut map: impl FnMut(&DocReference, &str) -> String,
    ) -> Option<String> {
        let mut names = self.names.iter();
        self.template.as_ref().map(|template| {
            template.render(|reference| {
                map(reference, names.next().expect("documentation was prepared"))
            })
        })
    }
}

impl From<SelectedTextSpec> for SelectedDocSpec {
    fn from(text: SelectedTextSpec) -> Self {
        Self::new(text.value.map(Into::into))
    }
}

impl TextSpec for SelectedDocSpec {
    fn is_empty(&self) -> bool {
        self.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_names_without_rewriting_literal_prose() {
        let mut doc = SelectedDocSpec::new(Some(DocTemplate::new(
            String::new(),
            vec![
                DocPart::Name(DocReference {
                    kind: ReferenceKind::Function,
                    identity: "service.start".into(),
                    name: "Start".into(),
                    parent: "Service".into(),
                }),
                DocPart::Text(" starts a workflow; SystemNexus_Start is literal.".into()),
            ],
        )));
        doc.prepare(|_| "SystemNexus_Start".into());
        assert_eq!(
            doc.value(),
            Some("SystemNexus_Start starts a workflow; SystemNexus_Start is literal.")
        );
        assert_eq!(
            doc.map_names(|name| name.strip_prefix("SystemNexus_").unwrap().into()),
            Some("Start starts a workflow; SystemNexus_Start is literal.".into())
        );
        assert!(doc.starts_with_name("SystemNexus_Start"));
    }
}
