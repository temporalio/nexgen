use super::*;

pub(super) fn context_propagation_error(operation: &str, reason: &str) -> Error {
    Error::InvalidWitDirective {
        path: PathBuf::from("<go-plan>"),
        context: format!("operation `{operation}`"),
        directive: "@nexus.context-propagation".to_string(),
        reason: reason.to_string(),
    }
}

pub(super) fn validate_context_propagation_mode(
    operation: &PlannedOperation,
    package: &GoPackageContext,
) -> Result<()> {
    if operation.context_propagation_header.value().is_some()
        && package.is_self_import("go.temporal.io/sdk/internal")
        && !package.workflow_interceptors
    {
        return Err(context_propagation_error(
            &operation.name,
            "requires Go workflow System Nexus generation into go.temporal.io/sdk/internal",
        ));
    }
    Ok(())
}

pub(super) fn context_propagation_header(
    operation: &PlannedOperation,
    spec: &PlannedSpec,
    package: &GoPackageContext,
) -> Result<Option<String>> {
    if !package.is_self_import("go.temporal.io/sdk/internal") {
        return Ok(None);
    }
    let Some(key) = operation.context_propagation_header.value() else {
        return Ok(None);
    };
    let record = operation
        .input
        .as_ref()
        .and_then(|input| planned_message_type(input, spec))
        .and_then(|input| record_for_message(spec, &input))
        .ok_or_else(|| {
            context_propagation_error(&operation.name, "requires a generated request record")
        })?;
    let field = record.fields.get(key).ok_or_else(|| {
        context_propagation_error(
            &operation.name,
            "header field is not present in the request",
        )
    })?;
    if !matches!(field.origin, Some(crate::spec::FieldOrigin::ApiOmitted)) {
        return Err(context_propagation_error(
            &operation.name,
            "header field must use @nexus.api-omit so context propagation owns its value",
        ));
    }
    Ok(Some(go_field_name(&field.name)))
}

pub(super) fn render_defaults(output: &mut String, params: &[RenderedUnpackedParam]) {
    for param in params
        .iter()
        .filter(|param| param_belongs_in_options(param))
    {
        if let Some(default) = &param.default_expression {
            let source = format!("opts.{}", param.field_name);
            let zero = param.default_punning_zero.as_deref().unwrap_or("nil");
            output.push_str(&format!("\tif {source} == {zero} {{\n"));
            if param.default_input.is_some() {
                output.push_str(&format!("\t\tvalue := {default}\n\t\t{source} = &value\n"));
            } else {
                output.push_str(&format!("\t\t{source} = {default}\n"));
            }
            output.push_str("\t}\n");
        }
        if let Some(fields) = &param.flattened_fields {
            // A flattened optional record only exists when at least one field
            // is supplied. Defaults must not create an otherwise absent record.
            let condition = fields
                .iter()
                .map(RenderedFlattenedField::presence_condition)
                .collect::<Vec<_>>()
                .join(" || ");
            if fields
                .iter()
                .any(|field| field.default_expression.is_some())
            {
                output.push_str(&format!("\tif {condition} {{\n"));
                for field in fields {
                    if let Some(default) = &field.default_expression {
                        let source = format!("opts.{}", field.field.name);
                        let zero = field.default_punning_zero.as_deref().unwrap_or("nil");
                        output.push_str(&format!(
                            "\t\tif {source} == {zero} {{\n\t\t\t{source} = {default}\n\t\t}}\n"
                        ));
                    }
                }
                output.push_str("\t}\n");
            }
        }
    }
}

fn call_arguments(operation: &RenderedOperation<'_>, resolved_functions: bool) -> Vec<String> {
    let mut arguments = vec!["ctx".to_string(), "opts".to_string()];
    if let Some(params) = &operation.unpacked_input {
        let positional = ordered_positional_params(params);
        for (index, param) in positional.iter().enumerate() {
            let name = if !resolved_functions
                && param
                    .function
                    .as_ref()
                    .is_some_and(|f| !function_value_is_string(f))
            {
                function_name_local_var_for_params(param, params)
            } else if has_explicit_args(param) {
                singular_args_public_param_name(param)
            } else {
                param.param_name.clone()
            };
            let variadic = index + 1 == positional.len()
                && param.function_args.as_ref().is_some_and(|f| {
                    !has_explicit_args(param) && matches!(f.args, FunctionArgsSpec::Varargs { .. })
                });
            arguments.push(format!("{name}{}", if variadic { "..." } else { "" }));
        }
    } else {
        arguments.push("request".to_string());
    }
    arguments
}

pub(super) fn render_call(
    output: &mut String,
    operation: &RenderedOperation<'_>,
    package: &GoPackageContext,
) {
    let method = go_field_name(operation.name);
    let arguments = call_arguments(operation, false).join(", ");
    if package.is_self_import("go.temporal.io/sdk/internal") {
        if operation.context_propagation_header.is_some() {
            output.push_str("\tctx = workflowContextWithNewHeader(ctx)\n");
        }
        output.push_str(&format!(
            "\treturn getWorkflowOutboundInterceptor(ctx).{method}({arguments})\n",
        ));
    } else {
        output.push_str(&format!(
            "\treturn {}(func(ctx {}, input {}) {} {{\n\
             \t\treturn input.Client.ExecuteOperation(ctx, input.Operation, input.Input, input.Options)\n\
             \t}}, {arguments})\n",
            go_declaration_name(&format!("WorkflowEnvironmentInterceptor_{method}")),
            package.workflow_context_type(),
            package.sdk_reference(SdkSymbol::ExecuteNexusOperationInput).expression,
            package.sdk_reference(SdkSymbol::NexusOperationFuture).expression,
        ));
    }
}

fn signature(
    operation: &RenderedOperation<'_>,
    package: &GoPackageContext,
    visibility: &GoVisibility,
    dependency: Option<&str>,
) -> String {
    let mut params = Vec::new();
    if let Some(dependency) = dependency {
        params.push(dependency.to_string());
    }
    params.extend([
        format!("ctx {}", package.workflow_context_type()),
        format!(
            "opts {}Options",
            go_declaration_name(&go_field_name(operation.name))
        ),
    ]);
    if let Some(unpacked) = &operation.unpacked_input {
        let positional = ordered_positional_params(unpacked);
        for (index, param) in positional.iter().enumerate() {
            let (name, ty) = if param.function.is_some() {
                (param.param_name.clone(), param.go_type.clone())
            } else {
                wrapper_param_signature(
                    param,
                    unpacked,
                    index + 1 == positional.len(),
                    package,
                    visibility,
                )
            };
            params.push(format!("{name} {ty}"));
        }
    } else {
        params.push(format!("request {}", operation.input_type));
    }
    format!("({}) {}", params.join(", "), package.workflow_future_type())
}

pub(super) fn render(
    services: &[RenderedService<'_>],
    package: &GoPackageContext,
    visibility: &GoVisibility,
) -> Result<String> {
    let operations = services
        .iter()
        .flat_map(|service| {
            service
                .operations
                .iter()
                .map(move |operation| (service, operation))
        })
        .collect::<Vec<_>>();
    let mut names = BTreeSet::new();
    for (_, operation) in &operations {
        if !operation.model_type_parameters.is_empty() {
            return Err(Error::UnsupportedGoType {
                context: format!("interceptor for `{}`", operation.name),
                reason: "Go interceptor methods require concrete types".to_string(),
            });
        }
        if !names.insert(go_field_name(operation.name)) {
            return Err(Error::UnsupportedGoType {
                context: format!("interceptor for `{}`", operation.name),
                reason: "operation names must be distinct across SDK interceptor services"
                    .to_string(),
            });
        }
    }

    let interface = go_declaration_name("workflowOutboundInterceptor");
    let next = format!("next {interface}");
    let execute_param = format!(
        "executeNexusOperation func(ctx {}, input {}) {}",
        package.workflow_context_type(),
        package
            .sdk_reference(SdkSymbol::ExecuteNexusOperationInput)
            .expression,
        package
            .sdk_reference(SdkSymbol::NexusOperationFuture)
            .expression,
    );
    let mut output = String::from("\n// --- Workflow outbound interceptors ---\n\n");
    package.render_declaration_doc(&mut output, "", &interface, None);
    output.push_str(&format!("type {interface} interface {{\n"));
    for (_, operation) in &operations {
        let method = go_field_name(operation.name);
        output.push_str(&format!("\t// {method} intercepts workflow.{method}.\n"));
        if operation.context_propagation_header.is_some() {
            output.push_str("\t// WorkflowHeader returns a non-nil map for this context.\n");
        }
        output.push_str(&format!(
            "\t{method}{}\n",
            signature(operation, package, visibility, None),
        ));
    }
    output.push_str("}\n");

    for (service, operation) in &operations {
        let method = go_field_name(operation.name);
        let arguments = call_arguments(operation, true).join(", ");
        let execute = if package.is_self_import("go.temporal.io/sdk/internal") {
            let method_signature = signature(operation, package, visibility, None);
            output.push_str(&format!(
                "\n// {method} implements WorkflowOutboundInterceptor.{method}.\n\
                 func (w *WorkflowOutboundInterceptorBase) {method}{method_signature} {{\n\
                 \treturn w.Next.{method}({arguments})\n}}\n\
                 \nfunc (wc *workflowEnvironmentInterceptor) {method}{method_signature} {{\n"
            ));
            "wc.ExecuteNexusOperation"
        } else {
            let base = go_declaration_name(&format!("WorkflowOutboundInterceptorBase_{method}"));
            let terminal = go_declaration_name(&format!("WorkflowEnvironmentInterceptor_{method}"));
            output.push_str(&format!(
                "\n// {base} forwards to the next interceptor.\n\
                 func {base}{} {{\n\treturn next.{method}({arguments})\n}}\n\
                 \n// {terminal} constructs and schedules the intercepted request.\n\
                 func {terminal}{} {{\n",
                signature(operation, package, visibility, Some(&next)),
                signature(operation, package, visibility, Some(&execute_param)),
            ));
            "executeNexusOperation"
        };
        if operation.context_propagation_header.is_some() {
            output.push_str(
                "\t_nexgenHeader, err := workflowHeaderPropagated(ctx, getWorkflowEnvOptions(ctx).ContextPropagators)\n\
                 \tif err != nil {\n\
                 \t\tresult, resultSettable := NewFuture(ctx)\n\
                 \t\tresultSettable.SetError(err)\n\
                 \t\treturn result\n\
                 \t}\n",
            );
        }
        if let Some(params) = &operation.unpacked_input {
            render_public_input_locals(&mut output, params, false);
            output.push_str("\trequest := ");
            render_operation_request(&mut output, operation, params, true);
            output.push('\n');
        } else {
            for (field, expression) in &operation.sourced_fields {
                output.push_str(&format!("\trequest.{field} = {expression}\n"));
            }
        }
        render_operation_body(
            &mut output,
            service,
            operation,
            package,
            OperationExecution::ExecuteNexusOperation(execute),
        );
        output.push_str("}\n");
    }
    Ok(output)
}
