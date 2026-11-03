use super::*;

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
    let mut body = String::new();
    let mut names = BTreeMap::new();
    for enumeration in enums
        .values()
        .filter(|value| is_go_exported_decl(&value.name))
    {
        render_alias(&mut body, &mut names, &enumeration.name);
        for value in &enumeration.values {
            render_constant(&mut body, &format!("{}{}", enumeration.name, value.name));
        }
    }
    for flags in flags
        .values()
        .filter(|value| is_go_exported_decl(&value.name))
    {
        render_alias(&mut body, &mut names, &flags.name);
        for flag in &flags.flags {
            render_constant(&mut body, &format!("{}{}", flags.name, flag.name));
        }
    }
    for variant in variants
        .values()
        .filter(|value| is_go_exported_decl(&value.name))
    {
        if let Some(doc) = variant.doc.value() {
            render_go_doc_comment(&mut body, "", doc);
        }
        let declaration = go_generic_declaration_name(&variant.name, &variant.type_parameters);
        render_alias(&mut body, &mut names, &declaration);
        for case in &variant.cases {
            let declaration =
                go_generic_declaration_name(&case.struct_name, &variant.type_parameters);
            render_alias(&mut body, &mut names, &declaration);
        }
    }
    for (key, model) in models
        .iter()
        .filter(|(_, model)| is_go_exported_type_decl(&model.name))
    {
        if let Some(record) = record_for_model_key(api_plan, key)
            && let Some(doc) = record.doc.value()
        {
            render_go_doc_comment(&mut body, "", doc);
        }
        render_alias(&mut body, &mut names, &model.name);
    }
    for service in services {
        for resource in &service.resources {
            render_alias(
                &mut body,
                &mut names,
                &go_declaration_name(&resource.type_name),
            );
        }
        if service.endpoint.is_some() {
            for operation in &service.operations {
                let declaration = go_generic_declaration_name(
                    &format!(
                        "{}Options",
                        go_declaration_name(&go_field_name(operation.name))
                    ),
                    &operation.model_type_parameters,
                );
                render_alias(&mut body, &mut names, &declaration);
            }
        }
    }

    for service in services {
        for resource in &service.resources {
            let params = resource
                .fields
                .iter()
                .map(|field| {
                    let kind = planned_field_kind(&field.kind, api_plan);
                    let ty = resolve_resource_field_kind(&kind, field.optional, package);
                    let ty = if field.optional
                        && public_default_punning_zero_for_field(&kind, &ty).is_some()
                    {
                        ty.trim_start_matches('*').to_string()
                    } else {
                        ty
                    };
                    (
                        go_unexported_name(&go_field_name(&field.name)),
                        visibility.rewrite_go_expr(&ty),
                    )
                })
                .collect::<Vec<_>>();
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
            render_function(
                &mut body,
                &format!("New{}", resource.type_name),
                &[],
                &params,
                &result,
                &names,
            );
        }
        if service.endpoint.is_none() {
            continue;
        }
        for operation in &service.operations {
            let name = go_field_name(operation.name);
            let mut params = vec![("ctx".to_string(), package.workflow_context_type())];
            let type_parameters;
            if let Some(unpacked) = &operation.unpacked_input {
                type_parameters = wrapper_type_parameters(operation, unpacked, package, visibility);
                params.push((
                    "opts".to_string(),
                    go_generic_reference(
                        &format!("{}Options", go_declaration_name(&name)),
                        &operation.model_type_parameters,
                        false,
                    ),
                ));
                let positional = ordered_positional_params(unpacked);
                let docs = wrapper_input_docs(positional.iter().copied());
                render_operation_doc_comment(&mut body, operation, &docs);
                params.extend(positional.iter().enumerate().map(|(index, param)| {
                    wrapper_param_signature(
                        param,
                        unpacked,
                        index + 1 == positional.len(),
                        package,
                        visibility,
                    )
                }));
            } else {
                type_parameters = Vec::new();
                render_operation_doc_comment(&mut body, operation, &[]);
                params.push((
                    "opts".to_string(),
                    format!("{}Options", go_declaration_name(&name)),
                ));
                params.push(("request".to_string(), operation.input_type.clone()));
            }
            render_function(
                &mut body,
                &name,
                &type_parameters,
                &params,
                &package.workflow_future_type(),
                &names,
            );
        }
    }

    let mut output = format!(
        "{GENERATED_HEADER}\n\npackage {}\n\nimport (\n\timplementation {:?}\n",
        go_package_name(&options.package_name),
        options.implementation_import_path,
    );
    let mut candidates = external_imports.to_vec();
    candidates.extend(imports.iter().map(|path| {
        (
            path.clone(),
            path.rsplit('/').next().unwrap_or(path).to_string(),
        )
    }));
    let mut imported = BTreeSet::from([options.implementation_import_path.clone()]);
    for (path, alias) in candidates {
        if body.contains(&format!("{alias}.")) && imported.insert(path.clone()) {
            output.push_str(&format!("\t{alias} {path:?}\n"));
        }
    }
    output.push_str(")\n\n");
    output.push_str(&body);
    output
}

fn public_name(name: &str) -> &str {
    name.strip_prefix("SystemNexus_").unwrap_or(name)
}

fn render_alias(output: &mut String, names: &mut BTreeMap<String, String>, declaration: &str) {
    let (name, suffix) = split_go_type_decl_name(declaration);
    let public = public_name(name);
    names.insert(name.to_string(), public.to_string());
    let arguments = suffix
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .map(|parameters| {
            let names = parameters
                .split(',')
                .map(|parameter| parameter.split_whitespace().next().unwrap())
                .collect::<Vec<_>>();
            format!("[{}]", names.join(", "))
        })
        .unwrap_or_default();
    output.push_str(&format!(
        "type {public}{suffix} = implementation.{name}{arguments}\n\n",
    ));
}

fn render_constant(output: &mut String, name: &str) {
    output.push_str(&format!(
        "const {} = implementation.{name}\n\n",
        public_name(name),
    ));
}

fn render_function(
    output: &mut String,
    name: &str,
    type_parameters: &[(String, String)],
    params: &[(String, String)],
    result: &str,
    names: &BTreeMap<String, String>,
) {
    let generics = type_parameters
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<BTreeSet<_>>();
    output.push_str(&format!("func {name}"));
    if !type_parameters.is_empty() {
        output.push('[');
        for (index, (name, constraint)) in type_parameters.iter().enumerate() {
            if index > 0 {
                output.push_str(", ");
            }
            output.push_str(&format!(
                "{name} {}",
                qualify_type(constraint, names, &generics)
            ));
        }
        output.push(']');
    }
    output.push_str("(\n");
    for (name, ty) in params {
        output.push_str(&format!(
            "\t{name} {},\n",
            qualify_type(ty, names, &generics)
        ));
    }
    output.push_str(&format!(
        ") {} {{\n\treturn implementation.{}",
        qualify_type(result, names, &generics),
        go_declaration_name(name),
    ));
    if !type_parameters.is_empty() {
        output.push('[');
        output.push_str(
            &type_parameters
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        );
        output.push(']');
    }
    output.push('(');
    for (index, (name, ty)) in params.iter().enumerate() {
        if index > 0 {
            output.push_str(", ");
        }
        output.push_str(name);
        if ty.starts_with("...") {
            output.push_str("...");
        }
    }
    output.push_str(")\n}\n\n");
}

fn qualify_type(expr: &str, names: &BTreeMap<String, String>, generics: &BTreeSet<&str>) -> String {
    let mut replacements = names.clone();
    for ident in expr.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_') {
        if is_go_exported_decl(ident) && !generics.contains(ident) {
            replacements
                .entry(ident.to_string())
                .or_insert_with(|| format!("implementation.{ident}"));
        }
    }
    GoVisibility {
        type_name_replacements: replacements,
        ..Default::default()
    }
    .rewrite_go_expr(expr)
}
