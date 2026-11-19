//! Prepare bound documentation using a backend-owned naming policy.

use crate::documentation::{DocReference, SelectedDocSpec};
use crate::planning::{PlannedFamily, PlannedSpec};
use crate::spec::{ApiSpecNode, ApiSpecTree, TypeDeclSpec};

pub(super) fn visit_docs(spec: &mut PlannedSpec, mut visit: impl FnMut(&mut SelectedDocSpec)) {
    for entry in spec.types.values_mut() {
        match &mut entry.declaration {
            TypeDeclSpec::Record(record) => {
                visit(&mut record.doc);
                for field in record.fields.values_mut() {
                    if let Some(doc) = &mut field.doc {
                        visit(doc);
                    }
                }
            }
            TypeDeclSpec::Variant(variant) => {
                visit(&mut variant.doc);
                for case in &mut variant.cases {
                    visit(&mut case.doc);
                }
            }
            _ => {}
        }
    }
    for service in &mut spec.services {
        visit(&mut service.doc);
        for operation in &mut service.operations {
            visit(&mut operation.doc);
            visit(&mut operation.return_doc);
        }
        for resource in &mut service.resources {
            visit(&mut resource.doc);
            visit(&mut resource.data.doc);
            for method in &mut resource.methods {
                visit(&mut method.doc);
            }
            for method in &mut resource.data.methods {
                visit(&mut method.doc);
            }
        }
    }
}

pub(super) fn prepare_tree(
    tree: &ApiSpecTree<PlannedFamily>,
    name: impl Fn(&DocReference, &PlannedSpec) -> String + Copy,
) -> ApiSpecTree<PlannedFamily> {
    fn visit(
        node: &mut ApiSpecNode<PlannedFamily>,
        name: impl Fn(&DocReference, &PlannedSpec) -> String + Copy,
    ) {
        match node {
            ApiSpecNode::Leaf(leaf) => {
                let source = leaf.spec.clone();
                visit_docs(&mut leaf.spec, |doc| {
                    doc.prepare(|reference| name(reference, &source));
                });
            }
            ApiSpecNode::Branch(branch) => {
                for child in branch.children.values_mut() {
                    visit(child, name);
                }
            }
        }
    }
    let mut tree = tree.clone();
    visit(&mut tree.root, name);
    tree
}

pub(super) fn type_name<'a>(reference: &'a DocReference, spec: &'a PlannedSpec) -> &'a str {
    match spec
        .types
        .get(&reference.identity)
        .map(|entry| &entry.declaration)
    {
        Some(TypeDeclSpec::Record(record)) => &record.name,
        Some(TypeDeclSpec::Enum(enumeration)) => &enumeration.name,
        Some(TypeDeclSpec::Flags(flags)) => &flags.name,
        Some(TypeDeclSpec::Variant(variant)) => &variant.name,
        _ => &reference.name,
    }
}
