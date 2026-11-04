use super::*;

pub(super) fn input_name(operation: &RenderedOperation<'_>) -> String {
    go_declaration_name(&format!("{}Input", go_field_name(operation.name)))
}

fn header_field(
    service: &RenderedService<'_>,
    operation: &RenderedOperation<'_>,
    api_plan: &PlannedSpec,
) -> Option<String> {
    let operation = api_plan
        .services
        .iter()
        .find(|planned| planned.wire_name == service.wire_name)?
        .operations
        .iter()
        .find(|planned| planned.name == operation.name)?;
    let input = planned_message_type(operation.input.as_ref()?, api_plan)?;
    let record = record_for_message(api_plan, &input)?;
    record.model_fields().find_map(|(_, field)| {
        let crate::planning::PlannedWireFieldBinding::Value { wire_type, .. } =
            field.data.wire_binding.as_ref()?
        else {
            return None;
        };
        let message = planned_message_type(wire_type, api_plan)?;
        (message.info.full_name == "temporal.api.common.v1.Header")
            .then(|| go_field_name(&field.name))
    })
}

pub(super) fn render(services: &[RenderedService<'_>], api_plan: &PlannedSpec) -> Result<String> {
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
                reason: "Go interceptor methods require a concrete request type".to_string(),
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

    let mut output = String::from("\n// --- Workflow outbound interceptors ---\n\n");
    output.push_str("type systemnexus_workflowOutboundInterceptor interface {\n");
    for (_, operation) in &operations {
        let method = go_field_name(operation.name);
        let input = input_name(operation);
        output.push_str(&format!(
            "\t// {method} intercepts the unencoded System Nexus request.\n\
             \t{method}(ctx Context, input *{input}) NexusOperationFuture\n"
        ));
    }
    output.push_str("}\n");

    for (service, operation) in &operations {
        let method = go_field_name(operation.name);
        let input = input_name(operation);
        let request = &operation.input_type;
        let service_name = go_string_literal(service.wire_name);
        let wire_name = go_string_literal(operation.wire_name);
        output.push_str(&format!(
            "\n// {input} contains the request before payload conversion.\n\
             // Interceptors may modify it synchronously before forwarding it. They must\n\
             // not retain or mutate it after forwarding, or replace its header map.\n\
             type {input} struct {{\n\
             \tRequest {request}\n\
             \tOptions NexusOperationOptions\n\
             \tNexusHeader nexus.Header\n\
             }}\n\
             \nfunc (w *WorkflowOutboundInterceptorBase) {method}(ctx Context, input *{input}) NexusOperationFuture {{\n\
             \treturn w.Next.{method}(ctx, input)\n\
             }}\n\
             \nfunc (wc *workflowEnvironmentInterceptor) {method}(ctx Context, input *{input}) NexusOperationFuture {{\n\
             \treturn wc.systemnexus_executeOperation(ctx, ExecuteNexusOperationInput{{\n\
             \t\tClient: NewSystemNexusClient({service_name}),\n\
             \t\tOperation: {wire_name},\n\
             \t\tInput: input.Request,\n\
             \t\tOptions: input.Options,\n\
             \t\tNexusHeader: input.NexusHeader,\n\
             \t}})\n\
             }}\n"
        ));
    }
    output.push_str(
        "\nfunc systemnexus_startOperation(ctx Context, outbound WorkflowOutboundInterceptor, input ExecuteNexusOperationInput) NexusOperationFuture {\n\
         \toperationName, err := nexusOperationName(input.Operation, input.Input)\n\
         \tif err != nil {\n\
         \t\treturn systemnexus_failedFuture(ctx, err)\n\
         \t}\n\
         \tswitch {\n",
    );
    for (service, operation) in operations {
        let method = go_field_name(operation.name);
        let input = input_name(operation);
        let request = &operation.input_type;
        let service_name = go_string_literal(service.wire_name);
        let wire_name = go_string_literal(operation.wire_name);
        let prepare_header = header_field(service, operation, api_plan)
            .map(|field| format!("\tctx = systemnexus_contextWithHeader(ctx, &request.{field})\n"))
            .unwrap_or_default();
        output.push_str(&format!(
            "\tcase input.Client.Service() == {service_name} && operationName == {wire_name}:\n\
             \trequest, ok := input.Input.({request})\n\
             \tif !ok {{\n\
             \t\treturn systemnexus_failedFuture(ctx, fmt.Errorf(\"system Nexus operation %s/%s expects {request}, got %T\", {service_name}, {wire_name}, input.Input))\n\
             \t}}\n\
             {prepare_header}\
             \treturn outbound.{method}(ctx, &{input}{{\n\
             \t\tRequest: request,\n\
             \t\tOptions: input.Options,\n\
             \t\tNexusHeader: input.NexusHeader,\n\
             \t}})\n"
        ));
    }
    output.push_str(
        "\t}\n\
         \treturn systemnexus_failedFuture(ctx, fmt.Errorf(\"unsupported system Nexus operation %s/%s\", input.Client.Service(), operationName))\n\
         }\n",
    );
    Ok(output)
}
