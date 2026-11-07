use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
struct TypeReference {
    expression: String,
    import: Option<(String, String)>,
}

impl TypeReference {
    fn implementation(name: &str, options: &GoFacadeOptions) -> Self {
        let qualifier = go_package_name(
            options
                .implementation_import_path
                .rsplit('/')
                .next()
                .unwrap(),
        );
        Self {
            expression: format!("{qualifier}.{name}"),
            import: Some((options.implementation_import_path.clone(), qualifier)),
        }
    }

    fn sdk(symbol: SdkSymbol, options: &GoFacadeOptions) -> Self {
        let reference = symbol.resolve(
            Some(&options.import_path),
            crate::nexgen_config::current().system_nexus,
        );
        Self {
            expression: reference.expression,
            import: reference
                .import
                .map(|(path, alias)| (path.to_string(), alias.to_string())),
        }
    }

    fn local(name: &str) -> Self {
        Self {
            expression: name.to_string(),
            import: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TypePart {
    Syntax(String),
    Reference(TypeReference),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TypePlan {
    parts: Vec<TypePart>,
}

impl TypePlan {
    fn expression(&self) -> String {
        self.parts
            .iter()
            .map(|part| match part {
                TypePart::Syntax(text) => text.as_str(),
                TypePart::Reference(reference) => reference.expression.as_str(),
            })
            .collect()
    }
}

struct TypeScope<'a> {
    options: &'a GoFacadeOptions,
    declarations: BTreeMap<String, TypeReference>,
    imports: BTreeMap<String, String>,
}

impl<'a> TypeScope<'a> {
    fn new(
        options: &'a GoFacadeOptions,
        package: &GoPackageContext,
        imports: &BTreeSet<String>,
        external_imports: &[(String, String)],
    ) -> Self {
        let mut scope = Self {
            options,
            declarations: BTreeMap::new(),
            imports: imports
                .iter()
                .map(|path| (package.import_alias(path).to_string(), path.clone()))
                .collect(),
        };
        for (path, alias) in external_imports {
            scope.imports.insert(
                alias.clone(),
                if package.is_self_import(path) {
                    options.implementation_import_path.clone()
                } else {
                    path.clone()
                },
            );
        }
        if package.import_path.is_some() {
            scope.imports.insert(
                package.package_name.clone(),
                options.implementation_import_path.clone(),
            );
        }
        for symbol in [SdkSymbol::Context, SdkSymbol::Future] {
            let reference = package.sdk_reference(symbol);
            if reference.import.is_none() {
                scope
                    .declarations
                    .insert(reference.expression, TypeReference::sdk(symbol, options));
            }
        }
        scope
    }

    // The Go backend represents resolved type expressions as strings. Bind their
    // named references against the declaration/import scope before rendering;
    // documentation and function bodies never enter this walk.
    fn resolve(&self, expression: &str, parameters: &BTreeSet<String>) -> TypePlan {
        let mut parts = Vec::new();
        let bytes = expression.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            let start = index;
            if matches!(bytes[index], b'"' | b'`') {
                let quote = bytes[index];
                index += 1;
                while index < bytes.len() {
                    let byte = bytes[index];
                    index += 1;
                    if byte == b'\\' && quote == b'"' && index < bytes.len() {
                        index += 1;
                    } else if byte == quote {
                        break;
                    }
                }
                parts.push(TypePart::Syntax(expression[start..index].to_string()));
            } else if is_go_ident_start(bytes[index]) {
                index += 1;
                while index < bytes.len() && is_go_ident_continue(bytes[index]) {
                    index += 1;
                }
                let name = &expression[start..index];
                if bytes.get(index) == Some(&b'.')
                    && bytes
                        .get(index + 1)
                        .is_some_and(|byte| is_go_ident_start(*byte))
                {
                    index += 1;
                    let member_start = index;
                    index += 1;
                    while index < bytes.len() && is_go_ident_continue(bytes[index]) {
                        index += 1;
                    }
                    if let Some(path) = self.imports.get(name) {
                        let member = &expression[member_start..index];
                        let reference = if let Some(symbol) = SdkSymbol::find(path, member) {
                            TypeReference::sdk(symbol, self.options)
                        } else if path == &self.options.import_path {
                            TypeReference::local(member)
                        } else if path == &self.options.implementation_import_path {
                            TypeReference::implementation(member, self.options)
                        } else {
                            TypeReference {
                                expression: format!("{name}.{member}"),
                                import: Some((path.clone(), name.to_string())),
                            }
                        };
                        parts.push(TypePart::Reference(reference));
                    } else {
                        parts.push(TypePart::Syntax(expression[start..index].to_string()));
                    }
                } else if !parameters.contains(name)
                    && let Some(reference) = self.declarations.get(name)
                {
                    parts.push(TypePart::Reference(reference.clone()));
                } else {
                    parts.push(TypePart::Syntax(name.to_string()));
                }
            } else {
                let ch = expression[index..].chars().next().unwrap();
                index += ch.len_utf8();
                parts.push(TypePart::Syntax(ch.to_string()));
            }
        }
        TypePlan { parts }
    }

    fn register_annotation(&mut self, annotation: &str, package: &GoPackageContext) {
        let (imports, expression) = parse_go_expr(annotation);
        if !imports.keys().any(|path| package.is_self_import(path)) {
            return;
        }
        let resolved = self.resolve(&expression, &BTreeSet::new());
        for part in resolved.parts {
            if let TypePart::Reference(reference) = part
                && let Some((path, _)) = &reference.import
                && path == &self.options.implementation_import_path
                && let Some((_, name)) = reference.expression.split_once('.')
            {
                self.declarations
                    .insert(name.to_string(), reference.clone());
            }
        }
    }

    fn register_authored_type(&mut self, kind: &PlannedType, package: &GoPackageContext) {
        match kind {
            TypeSpec::Option(inner) | TypeSpec::List(inner) => {
                self.register_authored_type(inner, package);
            }
            TypeSpec::Map(key, value) => {
                self.register_authored_type(key, package);
                self.register_authored_type(value, package);
            }
            TypeSpec::Tuple(items) => {
                for item in items {
                    self.register_authored_type(item, package);
                }
            }
            TypeSpec::Result { ok, err } => {
                for kind in [ok, err].into_iter().flatten() {
                    self.register_authored_type(kind, package);
                }
            }
            TypeSpec::External(ExternalTypeSpec::Alias(alias)) => {
                if let Some(annotation) = alias.type_name.value() {
                    self.register_annotation(annotation, package);
                } else {
                    self.register_authored_type(&alias.target, package);
                }
            }
            _ => {}
        }
    }

    fn register_function(&mut self, function: &FunctionFieldSpec, package: &GoPackageContext) {
        if let Some(descriptor) = &function.type_descriptor {
            for annotation in [&descriptor.value_type, &descriptor.args_type] {
                if let Some(annotation) = annotation.value() {
                    self.register_annotation(annotation, package);
                }
            }
        }
        let args = match &function.args {
            FunctionArgsSpec::Fixed(args) => args,
            FunctionArgsSpec::Varargs { prefix, .. } => prefix,
        };
        for arg in args {
            self.register_authored_type(&arg.field_type, package);
        }
        match &function.result {
            FunctionResultSpec::Authored(kind) => self.register_authored_type(kind, package),
            FunctionResultSpec::Annotation(annotation) => {
                if let Some(annotation) = annotation.value() {
                    self.register_annotation(annotation, package);
                }
            }
        }
        if let Some(alternate) = &function.alternate_type {
            self.register_authored_type(alternate, package);
        }
    }
}

struct AliasPlan {
    name: String,
    target: TypeReference,
    parameters: Vec<String>,
    documentation: Option<String>,
}

struct ConstantPlan {
    name: String,
    target: TypeReference,
}

struct ParameterPlan {
    name: String,
    kind: TypePlan,
    variadic: bool,
}

struct FunctionPlan {
    name: String,
    target: TypeReference,
    parameters: Vec<ParameterPlan>,
    type_parameters: Vec<(String, TypePlan)>,
    result: TypePlan,
    documentation: String,
}

struct FacadePlan {
    package_name: String,
    imports: BTreeMap<String, String>,
    aliases: Vec<AliasPlan>,
    constants: Vec<ConstantPlan>,
    functions: Vec<FunctionPlan>,
}

impl FacadePlan {
    fn record_reference(&mut self, reference: &TypeReference) {
        if let Some((path, alias)) = &reference.import {
            self.imports.insert(path.clone(), alias.clone());
        }
    }

    fn record_type(&mut self, kind: &TypePlan) {
        for part in &kind.parts {
            if let TypePart::Reference(reference) = part {
                self.record_reference(reference);
            }
        }
    }

    fn add_alias(
        &mut self,
        scope: &mut TypeScope<'_>,
        name: &str,
        parameters: Vec<String>,
        documentation: Option<&str>,
    ) {
        let name = split_go_type_decl_name(name).0;
        let public = public_name(name);
        scope
            .declarations
            .insert(name.to_string(), TypeReference::local(public));
        let target = TypeReference::implementation(name, scope.options);
        self.record_reference(&target);
        self.aliases.push(AliasPlan {
            name: public.to_string(),
            target,
            parameters,
            documentation: documentation.map(str::to_string),
        });
    }

    fn add_constant(&mut self, name: String, options: &GoFacadeOptions) {
        let target = TypeReference::implementation(&name, options);
        self.record_reference(&target);
        self.constants.push(ConstantPlan {
            name: public_name(&name).to_string(),
            target,
        });
    }

    fn add_function(&mut self, function: FunctionPlan) {
        self.record_reference(&function.target);
        for parameter in &function.parameters {
            self.record_type(&parameter.kind);
        }
        for (_, constraint) in &function.type_parameters {
            self.record_type(constraint);
        }
        self.record_type(&function.result);
        self.functions.push(function);
    }
}

pub(super) fn render(
    options: &GoFacadeOptions,
    package: &GoPackageContext,
    imports: &BTreeSet<String>,
    external_imports: &[(String, String)],
    enums: &IndexMap<String, RenderedEnum>,
    flags: &IndexMap<String, RenderedFlags>,
    variants: &IndexMap<String, RenderedVariant>,
    models: &IndexMap<String, RenderedModel>,
    services: &[RenderedService<'_>],
    api_plan: &PlannedSpec,
    visibility: &GoVisibility,
) -> String {
    let mut scope = TypeScope::new(options, package, imports, external_imports);
    let mut plan = FacadePlan {
        package_name: go_package_name(&options.package_name),
        imports: BTreeMap::new(),
        aliases: Vec::new(),
        constants: Vec::new(),
        functions: Vec::new(),
    };
    for enumeration in enums
        .values()
        .filter(|value| is_go_exported_decl(&value.name))
    {
        plan.add_alias(&mut scope, &enumeration.name, Vec::new(), None);
        for value in &enumeration.values {
            plan.add_constant(format!("{}{}", enumeration.name, value.name), options);
        }
    }
    for flags in flags
        .values()
        .filter(|value| is_go_exported_decl(&value.name))
    {
        plan.add_alias(&mut scope, &flags.name, Vec::new(), None);
        for flag in &flags.flags {
            plan.add_constant(format!("{}{}", flags.name, flag.name), options);
        }
    }
    for variant in variants
        .values()
        .filter(|value| is_go_exported_decl(&value.name))
    {
        let parameters = variant
            .type_parameters
            .iter()
            .map(|usage| usage.parameter.name.clone())
            .collect::<Vec<_>>();
        plan.add_alias(
            &mut scope,
            &variant.name,
            parameters.clone(),
            variant.doc.value(),
        );
        for case in &variant.cases {
            plan.add_alias(&mut scope, &case.struct_name, parameters.clone(), None);
        }
    }
    for (key, model) in models
        .iter()
        .filter(|(_, model)| is_go_exported_type_decl(&model.name))
    {
        plan.add_alias(
            &mut scope,
            &model.name,
            model.type_parameters.clone(),
            record_for_model_key(api_plan, key).and_then(|record| record.doc.value()),
        );
    }
    for service in services {
        for resource in &service.resources {
            plan.add_alias(
                &mut scope,
                &go_declaration_name(&resource.type_name),
                Vec::new(),
                None,
            );
        }
        if service.endpoint.is_some() {
            for operation in &service.operations {
                plan.add_alias(
                    &mut scope,
                    &format!(
                        "{}Options",
                        go_declaration_name(&go_field_name(operation.name))
                    ),
                    operation
                        .model_type_parameters
                        .iter()
                        .map(|usage| usage.parameter.name.clone())
                        .collect(),
                    None,
                );
            }
        }
    }
    for (_, record) in api_plan.records() {
        for field in record.fields.values() {
            if let Some(annotation) = field
                .flattened_annotation
                .as_ref()
                .or(field.annotation.as_ref())
                .and_then(|annotation| annotation.value())
            {
                scope.register_annotation(annotation, package);
            }
            scope.register_authored_type(&field.field_type, package);
            if let Some(function) = &field.function {
                scope.register_function(function, package);
            }
        }
    }

    for service in services {
        for resource in &service.resources {
            let parameters = resource
                .fields
                .iter()
                .map(|field| {
                    scope.register_authored_type(&field.kind, package);
                    let kind = planned_field_kind(&field.kind, api_plan);
                    let ty = resolve_resource_field_kind(&kind, field.optional, package);
                    let ty = if field.optional
                        && public_default_punning_zero_for_field(&kind, &ty).is_some()
                    {
                        ty.trim_start_matches('*').to_string()
                    } else {
                        ty
                    };
                    ParameterPlan {
                        name: go_unexported_name(&go_field_name(&field.name)),
                        kind: scope.resolve(&visibility.rewrite_go_expr(&ty), &BTreeSet::new()),
                        variadic: false,
                    }
                })
                .collect();
            let returns_value = service.operations.iter().any(|operation| {
                operation
                    .wire_binding
                    .as_ref()
                    .and_then(|binding| binding.returned_resource_type_name())
                    .is_some_and(|name| name == resource.type_name)
            });
            let result = format!(
                "{}{}",
                if returns_value { "" } else { "*" },
                go_declaration_name(&resource.type_name),
            );
            plan.add_function(FunctionPlan {
                name: format!("New{}", resource.type_name),
                target: TypeReference::implementation(
                    &go_declaration_name(&format!("New{}", resource.type_name)),
                    options,
                ),
                parameters,
                type_parameters: Vec::new(),
                result: scope.resolve(&result, &BTreeSet::new()),
                documentation: String::new(),
            });
        }
        if service.endpoint.is_none() {
            continue;
        }
        for operation in &service.operations {
            let name = go_field_name(operation.name);
            let type_parameters = operation
                .unpacked_input
                .as_ref()
                .map(|unpacked| wrapper_type_parameters(operation, unpacked, package, visibility))
                .unwrap_or_default();
            let parameter_names = type_parameters
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<BTreeSet<_>>();
            let mut parameters = vec![ParameterPlan {
                name: "ctx".to_string(),
                kind: scope.resolve(&package.workflow_context_type(), &parameter_names),
                variadic: false,
            }];
            let mut documentation = String::new();
            if let Some(unpacked) = &operation.unpacked_input {
                parameters.push(ParameterPlan {
                    name: "opts".to_string(),
                    kind: scope.resolve(
                        &go_generic_reference(
                            &format!("{}Options", go_declaration_name(&name)),
                            &operation.model_type_parameters,
                            false,
                        ),
                        &parameter_names,
                    ),
                    variadic: false,
                });
                let positional = ordered_positional_params(unpacked);
                let docs = wrapper_input_docs(positional.iter().copied());
                render_operation_doc_comment(&mut documentation, operation, &docs);
                for (index, param) in positional.iter().enumerate() {
                    let (name, kind) = wrapper_param_signature(
                        param,
                        unpacked,
                        index + 1 == positional.len(),
                        package,
                        visibility,
                    );
                    let variadic = index + 1 == positional.len()
                        && param.function_args.as_ref().is_some_and(|function| {
                            !has_explicit_args(param)
                                && matches!(function.args, FunctionArgsSpec::Varargs { .. })
                        });
                    parameters.push(ParameterPlan {
                        name,
                        kind: scope.resolve(
                            if variadic {
                                kind.strip_prefix("...").unwrap()
                            } else {
                                &kind
                            },
                            &parameter_names,
                        ),
                        variadic,
                    });
                }
            } else {
                render_operation_doc_comment(&mut documentation, operation, &[]);
                for (name, kind) in [
                    ("opts", format!("{}Options", go_declaration_name(&name))),
                    ("request", operation.input_type.clone()),
                ] {
                    parameters.push(ParameterPlan {
                        name: name.to_string(),
                        kind: scope.resolve(&kind, &parameter_names),
                        variadic: false,
                    });
                }
            }
            plan.add_function(FunctionPlan {
                name: name.clone(),
                target: TypeReference::implementation(&go_declaration_name(&name), options),
                parameters,
                type_parameters: type_parameters
                    .into_iter()
                    .map(|(name, constraint)| (name, scope.resolve(&constraint, &parameter_names)))
                    .collect(),
                result: scope.resolve(&package.workflow_future_type(), &parameter_names),
                documentation,
            });
        }
    }
    render_plan(&plan)
}

fn public_name(name: &str) -> &str {
    name.strip_prefix("SystemNexus_").unwrap_or(name)
}

fn render_plan(plan: &FacadePlan) -> String {
    let mut output = format!("{GENERATED_HEADER}\n\npackage {}\n", plan.package_name);
    if !plan.imports.is_empty() {
        output.push_str("\nimport (\n");
        for (path, alias) in &plan.imports {
            let package_name = go_package_name(path.rsplit('/').next().unwrap());
            if alias == &package_name {
                output.push_str(&format!("\t{path:?}\n"));
            } else {
                output.push_str(&format!("\t{alias} {path:?}\n"));
            }
        }
        output.push_str(")\n");
    }
    output.push('\n');
    for alias in &plan.aliases {
        if let Some(doc) = &alias.documentation {
            render_go_doc_comment(&mut output, "", doc);
        }
        output.push_str(&format!("type {}", alias.name));
        if !alias.parameters.is_empty() {
            output.push('[');
            output.push_str(
                &alias
                    .parameters
                    .iter()
                    .map(|name| format!("{name} any"))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            output.push(']');
        }
        output.push_str(&format!(" = {}", alias.target.expression));
        if !alias.parameters.is_empty() {
            output.push_str(&format!("[{}]", alias.parameters.join(", ")));
        }
        output.push_str("\n\n");
    }
    for constant in &plan.constants {
        output.push_str(&format!(
            "const {} = {}\n\n",
            constant.name, constant.target.expression,
        ));
    }
    for function in &plan.functions {
        output.push_str(&function.documentation);
        output.push_str(&format!("func {}", function.name));
        if !function.type_parameters.is_empty() {
            output.push('[');
            output.push_str(
                &function
                    .type_parameters
                    .iter()
                    .map(|(name, constraint)| format!("{name} {}", constraint.expression()))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            output.push(']');
        }
        output.push_str("(\n");
        for parameter in &function.parameters {
            output.push_str(&format!(
                "\t{} {}{},\n",
                parameter.name,
                if parameter.variadic { "..." } else { "" },
                parameter.kind.expression(),
            ));
        }
        output.push_str(&format!(
            ") {} {{\n\treturn {}",
            function.result.expression(),
            function.target.expression,
        ));
        if !function.type_parameters.is_empty() {
            output.push('[');
            output.push_str(
                &function
                    .type_parameters
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            output.push(']');
        }
        output.push('(');
        output.push_str(
            &function
                .parameters
                .iter()
                .map(|parameter| {
                    format!(
                        "{}{}",
                        parameter.name,
                        if parameter.variadic { "..." } else { "" },
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
        );
        output.push_str(")\n}\n\n");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> GoFacadeOptions {
        GoFacadeOptions {
            package_name: "workflow".to_string(),
            import_path: "example.org/sdk/workflow".to_string(),
            implementation_import_path: "example.org/sdk/internal".to_string(),
        }
    }

    fn package() -> GoPackageContext {
        GoPackageContext {
            package_name: "internal".to_string(),
            import_path: Some("example.org/sdk/internal".to_string()),
            workflow_package_alias: "workflow",
            has_serialization_context: false,
            support_package: None,
            facade: None,
            workflow_interceptors: false,
        }
    }

    #[test]
    fn signature_references_select_imports_from_their_scope() {
        let options = options();
        let mut scope = TypeScope::new(
            &options,
            &package(),
            &BTreeSet::from(["time".to_string(), "example.org/sdk/workflow".to_string()]),
            &[("example.org/enums/v1".to_string(), "enums".to_string())],
        );
        scope.declarations.insert(
            "SystemNexus_Result".to_string(),
            TypeReference::local("Result"),
        );
        let kind = scope.resolve(
            "func([]enums.Policy, time.Duration) *SystemNexus_Result",
            &BTreeSet::new(),
        );
        assert_eq!(
            kind.expression(),
            "func([]enums.Policy, time.Duration) *Result"
        );
        let imports = kind
            .parts
            .iter()
            .filter_map(|part| match part {
                TypePart::Reference(reference) => reference.import.clone(),
                TypePart::Syntax(_) => None,
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            imports,
            BTreeMap::from([
                ("example.org/enums/v1".to_string(), "enums".to_string()),
                ("time".to_string(), "time".to_string()),
            ]),
        );
    }

    #[test]
    fn documentation_does_not_participate_in_dependency_planning() {
        let options = options();
        let scope = TypeScope::new(
            &options,
            &package(),
            &BTreeSet::from([
                "example.org/sdk/workflow".to_string(),
                "example.org/sdk/converter".to_string(),
            ]),
            &[],
        );
        let mut plan = FacadePlan {
            package_name: "workflow".to_string(),
            imports: BTreeMap::new(),
            aliases: Vec::new(),
            constants: Vec::new(),
            functions: Vec::new(),
        };
        plan.add_function(FunctionPlan {
            name: "Call".to_string(),
            target: TypeReference::implementation("Call", &options),
            parameters: Vec::new(),
            type_parameters: Vec::new(),
            result: scope.resolve("any", &BTreeSet::new()),
            documentation: "Returns the started workflow. Uses converter.\n".to_string(),
        });
        assert_eq!(
            plan.imports,
            BTreeMap::from([(options.implementation_import_path, "internal".to_string(),)]),
        );
    }

    #[test]
    fn type_parameters_shadow_declared_names_without_case_based_inference() {
        let options = options();
        let mut scope = TypeScope::new(&options, &package(), &BTreeSet::new(), &[]);
        scope
            .declarations
            .insert("T".to_string(), TypeReference::local("AliasT"));
        scope.declarations.insert(
            "SystemNexus_Result".to_string(),
            TypeReference::local("Result"),
        );
        let kind = scope.resolve(
            "func(T) SystemNexus_Result[T]",
            &BTreeSet::from(["T".to_string()]),
        );
        assert_eq!(kind.expression(), "func(T) Result[T]");
    }

    #[test]
    fn authored_self_types_retain_their_implementation_dependency() {
        let options = options();
        let mut scope = TypeScope::new(&options, &package(), &BTreeSet::new(), &[]);
        scope.register_annotation(
            "example.org/sdk/internal:map[string]*internal.Custom",
            &package(),
        );
        assert_eq!(
            scope.resolve("*Custom", &BTreeSet::new()).parts,
            vec![
                TypePart::Syntax("*".to_string()),
                TypePart::Reference(TypeReference::implementation("Custom", &options)),
            ],
        );
    }

    #[test]
    fn sdk_workflow_facade_uses_existing_local_types() {
        let _config = crate::nexgen_config::scope(crate::nexgen_config::NexgenConfig {
            system_nexus: true,
            targets: vec![],
        });
        let options = GoFacadeOptions {
            package_name: "workflow".to_string(),
            import_path: "go.temporal.io/sdk/workflow".to_string(),
            implementation_import_path: "go.temporal.io/sdk/internal".to_string(),
        };
        let mut package = package();
        package.import_path = Some(options.implementation_import_path.clone());
        let scope = TypeScope::new(&options, &package, &BTreeSet::new(), &[]);
        for (expression, name) in [
            ("Context", "Context"),
            ("Future", "Future"),
            ("internal.Context", "Context"),
            ("internal.Future", "Future"),
        ] {
            assert_eq!(
                scope.resolve(expression, &BTreeSet::new()).parts,
                vec![TypePart::Reference(TypeReference::local(name))],
            );
        }
        assert_eq!(
            scope.resolve("internal.Custom", &BTreeSet::new()).parts,
            vec![TypePart::Reference(TypeReference::implementation(
                "Custom", &options
            ))],
        );
    }

    #[test]
    fn other_workflow_packages_keep_sdk_qualifiers() {
        let _config = crate::nexgen_config::scope(crate::nexgen_config::NexgenConfig {
            system_nexus: true,
            targets: vec![],
        });
        let mut options = options();
        options.implementation_import_path = "go.temporal.io/sdk/internal".to_string();
        let mut package = package();
        package.import_path = Some(options.implementation_import_path.clone());
        let scope = TypeScope::new(&options, &package, &BTreeSet::new(), &[]);
        assert_eq!(
            scope.resolve("internal.Future", &BTreeSet::new()).parts,
            vec![TypePart::Reference(TypeReference {
                expression: "internal.Future".to_string(),
                import: Some((
                    "go.temporal.io/sdk/internal".to_string(),
                    "internal".to_string()
                )),
            })],
        );
    }
}
