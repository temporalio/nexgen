//! Bind authored names while the WIT resolver and lexical scopes are available.

use crate::documentation::{DocPart, DocReference, DocTemplate, LanguageDocSpec, ReferenceKind};

use super::*;

struct Scope<'a> {
    resolve: &'a Resolve,
    interface: &'a Interface,
    ty: Option<TypeId>,
    function: Option<&'a Function>,
    language: Language,
}

pub(super) fn for_type(
    resolve: &Resolve,
    id: TypeId,
    text: LanguageStringSpec,
    path: &Path,
    context: &str,
    language: Language,
) -> Result<LanguageDocSpec> {
    let TypeOwner::Interface(interface) = resolve.types[id].owner else {
        return Ok(text.into());
    };
    Scope {
        resolve,
        interface: &resolve.interfaces[interface],
        ty: Some(id),
        function: None,
        language,
    }
    .documentation(text, path, context)
}

pub(super) fn for_function(
    resolve: &Resolve,
    interface: &Interface,
    function: &Function,
    text: LanguageStringSpec,
    path: &Path,
    context: &str,
    language: Language,
) -> Result<LanguageDocSpec> {
    Scope {
        resolve,
        interface,
        ty: match function.kind {
            FunctionKind::Method(id) | FunctionKind::AsyncMethod(id) => Some(id),
            _ => None,
        },
        function: Some(function),
        language,
    }
    .documentation(text, path, context)
}

impl Scope<'_> {
    fn documentation(
        &self,
        text: LanguageStringSpec,
        path: &Path,
        context: &str,
    ) -> Result<LanguageDocSpec> {
        Ok(LanguageDocSpec {
            default: text
                .default
                .map(|text| self.template(text, path, context))
                .transpose()?,
            by_language: text
                .by_language
                .into_iter()
                .map(|(language, text)| Ok((language, self.template(text, path, context)?)))
                .collect::<Result<_>>()?,
        })
    }

    fn template(&self, text: String, path: &Path, context: &str) -> Result<DocTemplate> {
        let invalid = |reason| Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.doc".to_string(),
            reason,
        };
        let mut remaining = text.as_str();
        let mut parts = Vec::new();
        while let Some(start) = remaining.find("{@name") {
            // Doubling the opening brace quotes a marker, including its body.
            if start > 0 && remaining.as_bytes()[start - 1] == b'{' {
                let end = remaining[start..].find('}').ok_or_else(|| {
                    invalid("unterminated escaped documentation name reference".to_string())
                })? + start;
                parts.push(DocPart::Text(format!(
                    "{}{}",
                    &remaining[..start - 1],
                    &remaining[start..=end]
                )));
                remaining = &remaining[end + 1..];
                continue;
            }
            parts.push(DocPart::Text(remaining[..start].to_string()));
            let rest = &remaining[start + "{@name".len()..];
            let end = rest.find('}').ok_or_else(|| {
                invalid("unterminated documentation name reference; expected `}`".to_string())
            })?;
            let name = rest[..end].trim();
            if !rest.starts_with(char::is_whitespace)
                || name.is_empty()
                || name.chars().any(char::is_whitespace)
            {
                return Err(invalid(
                    "expected a documentation name reference of the form `{@name name}`"
                        .to_string(),
                ));
            }
            let reference = self
                .lookup(name)
                .ok_or_else(|| invalid(format!("unknown documentation name reference `{name}`")))?;
            parts.push(DocPart::Name(reference));
            remaining = &rest[end + 1..];
        }
        parts.push(DocPart::Text(remaining.to_string()));
        Ok(DocTemplate::new(text, parts))
    }

    fn lookup(&self, name: &str) -> Option<DocReference> {
        if let Some((interface, name)) = name.split_once('/') {
            let package = &self.resolve.packages[self.interface.package?];
            let interface = &self.resolve.interfaces[*package.interfaces.get(interface)?];
            return self.in_interface(interface, name);
        }
        if !name.contains('.') {
            if let Some(function) = self.function {
                if let Some(reference) = self.parameter(self.interface, function, name) {
                    return Some(reference);
                }
                if function.item_name() == name {
                    return Some(self.function_reference(self.interface, function));
                }
            }
            if let Some(ty) = self.ty {
                if let Some(reference) = self.member(ty, name) {
                    return Some(reference);
                }
            }
        }
        self.in_interface(self.interface, name)
    }

    fn in_interface(&self, interface: &Interface, name: &str) -> Option<DocReference> {
        let (root, member) = name
            .split_once('.')
            .map_or((name, None), |(root, member)| (root, Some(member)));
        if let Some(&id) = interface.types.get(root) {
            return match member {
                Some(member) => self.member(id, member),
                None => self.type_reference(id),
            };
        }
        let function = interface.functions.get(root)?;
        match member {
            Some(parameter) => self.parameter(interface, function, parameter),
            None => Some(self.function_reference(interface, function)),
        }
    }

    fn type_reference(&self, id: TypeId) -> Option<DocReference> {
        let ty = &self.resolve.types[id];
        let directives = parse_directives(
            ty.docs.contents.as_deref(),
            Path::new("<documentation>"),
            "type",
        )
        .ok()?;
        // Type replacements are already authored target expressions. Preserve
        // them for the backend rather than casing them as declaration names.
        if let Some(name) = directives
            .iter()
            .find(|directive| directive.name() == "type")
            .and_then(|directive| directive_language_value(directive, self.language))
        {
            return Some(DocReference {
                kind: ReferenceKind::TypeExpression,
                identity: wit_type_full_name(self.resolve, id),
                name: name.to_string(),
                parent: String::new(),
            });
        }
        if let TypeDefKind::Type(Type::Id(target)) = ty.kind {
            return self.type_reference(target);
        }
        Some(DocReference {
            kind: ReferenceKind::Type,
            identity: wit_type_full_name(self.resolve, id),
            name: ty.name.as_deref()?.to_upper_camel_case(),
            parent: String::new(),
        })
    }

    fn member(&self, mut id: TypeId, name: &str) -> Option<DocReference> {
        // Imported aliases retain the defining declaration's identity.
        while let TypeDefKind::Type(Type::Id(target)) = self.resolve.types[id].kind {
            id = target;
        }
        let ty = &self.resolve.types[id];
        let parent = ty.name.as_deref()?.to_upper_camel_case();
        let identity = wit_type_full_name(self.resolve, id);
        match &ty.kind {
            TypeDefKind::Record(record) => {
                let field = record.fields.iter().find(|field| field.name == name)?;
                let directives = parse_directives(
                    field.docs.contents.as_deref(),
                    Path::new("<documentation>"),
                    "field",
                )
                .ok()?;
                let name = directives
                    .iter()
                    .find(|directive| directive.name() == "name")
                    .and_then(|directive| directive_language_value(directive, self.language))
                    .unwrap_or(&field.name)
                    .to_string();
                Some(DocReference {
                    kind: ReferenceKind::Field,
                    identity,
                    name,
                    parent,
                })
            }
            TypeDefKind::Variant(variant) => {
                variant.cases.iter().find(|case| case.name == name)?;
                Some(DocReference {
                    kind: ReferenceKind::VariantCase,
                    identity,
                    name: name.to_string(),
                    parent,
                })
            }
            TypeDefKind::Resource => {
                let TypeOwner::Interface(interface) = ty.owner else {
                    return None;
                };
                let (method, parameter) = name
                    .split_once('.')
                    .map_or((name, None), |(method, parameter)| {
                        (method, Some(parameter))
                    });
                let function = self.resolve.interfaces[interface]
                    .functions
                    .values()
                    .find(|function| {
                        matches!(function.kind, FunctionKind::Method(owner) | FunctionKind::AsyncMethod(owner) if owner == id)
                            && function.item_name() == method
                    })?;
                parameter.map_or_else(
                    || Some(self.function_reference(&self.resolve.interfaces[interface], function)),
                    |name| self.parameter(&self.resolve.interfaces[interface], function, name),
                )
            }
            _ => None,
        }
    }

    fn function_reference(&self, interface: &Interface, function: &Function) -> DocReference {
        DocReference {
            kind: if matches!(
                function.kind,
                FunctionKind::Method(_) | FunctionKind::AsyncMethod(_)
            ) {
                ReferenceKind::Method
            } else {
                ReferenceKind::Function
            },
            identity: self.function_identity(interface, function),
            name: function.item_name().to_upper_camel_case(),
            parent: interface
                .name
                .as_deref()
                .unwrap_or_default()
                .to_upper_camel_case(),
        }
    }

    fn function_identity(&self, interface: &Interface, function: &Function) -> String {
        format!(
            "{}.{}.{}",
            interface
                .package
                .map(|id| self.resolve.packages[id].name.to_string())
                .unwrap_or_default(),
            interface.name.as_deref().unwrap_or_default(),
            function.name,
        )
    }

    fn parameter(
        &self,
        interface: &Interface,
        function: &Function,
        name: &str,
    ) -> Option<DocReference> {
        function.params.iter().find(|param| param.name == name)?;
        Some(DocReference {
            kind: if matches!(
                function.kind,
                FunctionKind::Freestanding | FunctionKind::AsyncFreestanding
            ) && !parse_directives(
                function.docs.contents.as_deref(),
                Path::new("<documentation>"),
                "function",
            )
            .ok()?
            .iter()
            .any(|directive| directive.name() == "function-args")
            {
                ReferenceKind::OperationParameter
            } else {
                ReferenceKind::Parameter
            },
            identity: self.function_identity(interface, function),
            name: name.to_string(),
            parent: function.item_name().to_upper_camel_case(),
        })
    }
}

pub(super) fn validate(
    resolve: &Resolve,
    path_for: impl Fn(Option<PackageId>) -> PathBuf,
    language: Language,
) -> Result<()> {
    for (_, interface) in resolve.interfaces.iter() {
        let path = path_for(interface.package);
        for function in interface.functions.values() {
            let context = format!("function `{}`", function.name);
            let directives = parse_directives(function.docs.contents.as_deref(), &path, &context)?;
            if let Some(doc) = directive(&directives, "doc", &path, &context)? {
                for text in [
                    directive_language_string(doc),
                    directive_returns_language_string(doc),
                ] {
                    for_function(
                        resolve, interface, function, text, &path, &context, language,
                    )?;
                }
            }
        }
    }
    for (id, ty) in resolve.types.iter() {
        let TypeOwner::Interface(interface) = ty.owner else {
            continue;
        };
        let path = path_for(resolve.interfaces[interface].package);
        let context = format!("type `{}`", wit_type_full_name(resolve, id));
        let validate = |docs: Option<&str>, context: &str| -> Result<()> {
            let directives = parse_directives(docs, &path, context)?;
            if let Some(doc) = directive(&directives, "doc", &path, context)? {
                for_type(
                    resolve,
                    id,
                    directive_language_string(doc),
                    &path,
                    context,
                    language,
                )?;
            }
            Ok(())
        };
        validate(ty.docs.contents.as_deref(), &context)?;
        match &ty.kind {
            TypeDefKind::Record(record) => {
                for field in &record.fields {
                    validate(
                        field.docs.contents.as_deref(),
                        &format!("{context} field `{}`", field.name),
                    )?;
                }
            }
            TypeDefKind::Variant(variant) => {
                for case in &variant.cases {
                    validate(
                        case.docs.contents.as_deref(),
                        &format!("{context} case `{}`", case.name),
                    )?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn showcase() -> ParsedWitPackage {
        parse_wit_with_inputs(
            include_str!("../../../advanced/samples/inputs/type-showcase.wit"),
            Path::new("type-showcase.wit"),
            &[],
        )
        .unwrap()
    }

    #[test]
    fn binds_types_functions_fields_and_parameters_before_casing() {
        let parsed = showcase();
        let resolve = &parsed.resolve;
        let interface =
            &resolve.interfaces[resolve.packages[parsed.package_id].interfaces["type-showcase"]];
        let scope = Scope {
            resolve,
            interface,
            ty: Some(interface.types["user-profile"]),
            function: None,
            language: Language::Go,
        };
        for (path, kind, name) in [
            ("tags", ReferenceKind::Field, "tags"),
            ("user-profile.tags", ReferenceKind::Field, "tags"),
            ("get-user-request.user-id", ReferenceKind::Field, "user-id"),
            (
                "type-showcase/get-user-request.user-id",
                ReferenceKind::Field,
                "user-id",
            ),
            ("user-profile", ReferenceKind::Type, "UserProfile"),
            ("update-email", ReferenceKind::Function, "UpdateEmail"),
            (
                "get-user.request",
                ReferenceKind::OperationParameter,
                "request",
            ),
            ("user.rename", ReferenceKind::Method, "Rename"),
            (
                "user.rename.display-name",
                ReferenceKind::Parameter,
                "display-name",
            ),
            (
                "notification-target.email",
                ReferenceKind::VariantCase,
                "email",
            ),
        ] {
            let reference = scope
                .lookup(path)
                .unwrap_or_else(|| panic!("reference `{path}`"));
            assert_eq!(reference.kind, kind, "{path}");
            assert_eq!(reference.name, name, "{path}");
        }
        assert_eq!(
            scope.lookup("tags").unwrap(),
            scope.lookup("user-profile.tags").unwrap()
        );
    }

    #[test]
    fn function_scope_binds_local_parameters_and_self() {
        let parsed = showcase();
        let resolve = &parsed.resolve;
        let interface =
            &resolve.interfaces[resolve.packages[parsed.package_id].interfaces["type-showcase"]];
        let scope = Scope {
            resolve,
            interface,
            ty: None,
            function: Some(&interface.functions["get-user"]),
            language: Language::Python,
        };
        assert_eq!(scope.lookup("request"), scope.lookup("get-user.request"));
        assert_eq!(
            scope.lookup("get-user").unwrap().kind,
            ReferenceKind::Function
        );
        assert!(scope.lookup("get-user.missing").is_none());
        assert!(scope.lookup("missing").is_none());
    }

    #[test]
    fn template_preserves_literal_names_and_quoted_markers() {
        let parsed = showcase();
        let resolve = &parsed.resolve;
        let interface =
            &resolve.interfaces[resolve.packages[parsed.package_id].interfaces["type-showcase"]];
        let scope = Scope {
            resolve,
            interface,
            ty: None,
            function: None,
            language: Language::Go,
        };
        let template = scope.template(
            "get-user stays literal; {{@name get-user} stays literal; {@name get-user} resolves.".into(),
            Path::new("test.wit"),
            "test",
        ).unwrap();
        assert_eq!(
            template.render(|reference| reference.name.clone()),
            "get-user stays literal; {@name get-user} stays literal; GetUser resolves."
        );
    }

    #[test]
    fn imported_types_and_renamed_fields_bind_to_their_definitions() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("advanced/samples/inputs/workflow-service.wit");
        let parsed = parse_wit_with_inputs(
            &fs::read_to_string(&path).unwrap(),
            &path,
            &[path.parent().unwrap().join("deps")],
        )
        .unwrap();
        let resolve = &parsed.resolve;
        let interface =
            &resolve.interfaces[resolve.packages[parsed.package_id].interfaces["workflow-service"]];
        let scope = Scope {
            resolve,
            interface,
            ty: Some(interface.types["signal-with-start-workflow-request"]),
            function: None,
            language: Language::Go,
        };
        let renamed = scope.lookup("execution-timeout").unwrap();
        assert_eq!(renamed.kind, ReferenceKind::Field);
        assert_eq!(renamed.name, "WorkflowExecutionTimeout");
        assert_eq!(
            scope.lookup("duration").unwrap().kind,
            ReferenceKind::TypeExpression
        );
        assert_eq!(
            scope.lookup("user-metadata").unwrap().kind,
            ReferenceKind::Field
        );
        assert_eq!(
            scope.lookup("workflow-service/user-metadata").unwrap().name,
            "UserMetadata"
        );
        assert_eq!(
            scope.lookup("user-metadata.static-summary").unwrap().name,
            "static-summary"
        );
    }
}
