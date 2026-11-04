use super::*;

pub(super) const STANDALONE_TYPES: [&str; 4] = [
    "SystemNexus_WorkflowOutboundInterceptor",
    "SystemNexus_WorkflowOutboundInterceptorBase",
    "SystemNexus_WorkflowOutboundInterceptorTerminal",
    "SystemNexus_WorkflowOutboundRuntime",
];

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

pub(super) fn has_headers(services: &[RenderedService<'_>], api_plan: &PlannedSpec) -> bool {
    services.iter().any(|service| {
        service
            .operations
            .iter()
            .any(|operation| header_field(service, operation, api_plan).is_some())
    })
}

pub(super) fn render(
    services: &[RenderedService<'_>],
    api_plan: &PlannedSpec,
    package: &GoPackageContext,
    header_payload_type: Option<&str>,
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

    let sdk = package.is_self_import("go.temporal.io/sdk/internal");
    let context = package.workflow_context_type();
    let future = package.workflow_reference("NexusOperationFuture");
    let options = package.workflow_reference("NexusOperationOptions");
    let execute_input = package.workflow_reference("ExecuteNexusOperationInput");
    let new_client = package.workflow_reference("NewSystemNexusClient");
    let [
        standalone_interface,
        standalone_base,
        standalone_terminal,
        runtime,
    ] = STANDALONE_TYPES;
    let (interface, base, terminal) = if sdk {
        (
            "systemnexus_workflowOutboundInterceptor",
            "WorkflowOutboundInterceptorBase",
            "workflowEnvironmentInterceptor",
        )
    } else {
        (standalone_interface, standalone_base, standalone_terminal)
    };
    let (execute, resolve, failed, bind_header) = if sdk {
        (
            "wc.systemnexus_executeOperation",
            "nexusOperationName",
            "systemnexus_failedFuture",
            "systemnexus_contextWithHeader",
        )
    } else {
        (
            "wc.Runtime.ExecuteOperation",
            "wc.Runtime.ResolveOperationName",
            "wc.Runtime.FailedFuture",
            "wc.Runtime.ContextWithHeader",
        )
    };

    let mut output = String::from("\n// --- Workflow outbound interceptors ---\n\n");
    output.push_str(&format!(
        "// {interface} intercepts System Nexus workflow operations before payload conversion.\n\
         type {interface} interface {{\n"
    ));
    for (_, operation) in &operations {
        let method = go_field_name(operation.name);
        let input = input_name(operation);
        output.push_str(&format!(
            "\t// {method} intercepts the unencoded System Nexus request.\n\
             \t{method}(ctx {context}, input *{input}) {future}\n"
        ));
    }
    output.push_str("}\n");

    if !sdk {
        output.push_str(&format!(
            "\n// {base} forwards operations to Next.\n\
             type {base} struct {{\n\
             \tNext {interface}\n\
             }}\n\
             \n// {terminal} schedules operations through Runtime without re-entering the interceptor chain.\n\
             type {terminal} struct {{\n\
             \tRuntime {runtime}\n\
             }}\n\
             \n// {runtime} supplies the host's operation validation,\n\
             // header context, failed futures, and terminal scheduling. ExecuteOperation\n\
             // must not dispatch through the same interceptor chain again. FailedFuture\n\
             // must fail both the result future and the operation-execution future.\n\
             type {runtime} interface {{\n\
             \tResolveOperationName(operation, input any) (string, error)\n\
             \tFailedFuture(ctx {context}, err error) {future}\n\
             \tExecuteOperation(ctx {context}, input {execute_input}) {future}\n"
        ));
        if let Some(payload) = header_payload_type {
            output.push_str(&format!(
                "\t// ContextWithHeader initializes a nil map and binds that same map to the returned context.\n\
                 \tContextWithHeader(ctx {context}, header *map[string]*{payload}) {context}\n"
            ));
        }
        output.push_str("}\n");
    }

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
             \tOptions {options}\n\
             \tNexusHeader nexus.Header\n\
             }}\n\
             \nfunc (w *{base}) {method}(ctx {context}, input *{input}) {future} {{\n\
             \treturn w.Next.{method}(ctx, input)\n\
             }}\n\
             \nfunc (wc *{terminal}) {method}(ctx {context}, input *{input}) {future} {{\n\
             \treturn {execute}(ctx, {execute_input}{{\n\
             \t\tClient: {new_client}({service_name}),\n\
             \t\tOperation: {wire_name},\n\
             \t\tInput: input.Request,\n\
             \t\tOptions: input.Options,\n\
             \t\tNexusHeader: input.NexusHeader,\n\
             \t}})\n\
             }}\n"
        ));
    }
    if sdk {
        output.push_str("\nfunc systemnexus_startOperation(");
    } else {
        output.push_str(&format!(
            "\n// StartOperation dispatches an unencoded request through outbound.\n\
             func (wc *{terminal}) StartOperation("
        ));
    }
    output.push_str(&format!(
        "ctx {context}, outbound {interface}, input {execute_input}) {future} {{\n\
         \toperationName, err := {resolve}(input.Operation, input.Input)\n\
         \tif err != nil {{\n\
         \t\treturn {failed}(ctx, err)\n\
         \t}}\n\
         \tswitch {{\n"
    ));
    for (service, operation) in operations {
        let method = go_field_name(operation.name);
        let input = input_name(operation);
        let request = &operation.input_type;
        let service_name = go_string_literal(service.wire_name);
        let wire_name = go_string_literal(operation.wire_name);
        let prepare_header = header_field(service, operation, api_plan)
            .map(|field| format!("\tctx = {bind_header}(ctx, &request.{field})\n"))
            .unwrap_or_default();
        output.push_str(&format!(
            "\tcase input.Client.Service() == {service_name} && operationName == {wire_name}:\n\
             \trequest, ok := input.Input.({request})\n\
             \tif !ok {{\n\
             \t\treturn {failed}(ctx, fmt.Errorf(\"system Nexus operation %s/%s expects {request}, got %T\", {service_name}, {wire_name}, input.Input))\n\
             \t}}\n\
             {prepare_header}\
             \treturn outbound.{method}(ctx, &{input}{{\n\
             \t\tRequest: request,\n\
             \t\tOptions: input.Options,\n\
             \t\tNexusHeader: input.NexusHeader,\n\
             \t}})\n"
        ));
    }
    output.push_str(&format!(
        "\t}}\n\
         \treturn {failed}(ctx, fmt.Errorf(\"unsupported system Nexus operation %s/%s\", input.Client.Service(), operationName))\n\
         }}\n"
    ));
    Ok(output)
}
