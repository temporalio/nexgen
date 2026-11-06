use super::*;

pub(super) const STANDALONE_TYPES: [&str; 5] = [
    "SystemNexus_WorkflowOutboundInterceptor",
    "SystemNexus_WorkflowOutboundInterceptorBase",
    "SystemNexus_WorkflowOutboundInterceptorTerminal",
    "SystemNexus_WorkflowOutboundRuntime",
    "SystemNexus_ExecuteOperationInput",
];

pub(super) fn render(
    services: &[RenderedService<'_>],
    package: &GoPackageContext,
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
    let execute_input = package.workflow_reference("ExecuteNexusOperationInput");
    let new_client = package.workflow_reference("NewSystemNexusClient");
    let [
        standalone_interface,
        standalone_base,
        standalone_terminal,
        runtime,
        standalone_dispatch_input,
    ] = STANDALONE_TYPES;
    let (interface, base, terminal, dispatch_input, failed) = if sdk {
        (
            "systemnexus_workflowOutboundInterceptor",
            "WorkflowOutboundInterceptorBase",
            "workflowEnvironmentInterceptor",
            "executeSystemNexusOperationInput",
            "FailedNexusOperationFuture",
        )
    } else {
        (
            standalone_interface,
            standalone_base,
            standalone_terminal,
            standalone_dispatch_input,
            "wc.FailedFuture",
        )
    };

    let mut output = String::from("\n// --- Workflow outbound interceptors ---\n\n");
    if sdk {
        output.push_str("func systemnexus_executeOperation(");
    } else {
        output.push_str(&format!(
            "func (wc *{terminal}) SystemNexus_ExecuteOperation("
        ));
    }
    output.push_str(&format!(
        "ctx {context}, outbound {interface}, input {dispatch_input}) {future} {{\n\
         \tswitch {{\n"
    ));
    for (service, operation) in &operations {
        let method = go_field_name(operation.name);
        let request = &operation.input_type;
        let service_name = go_string_literal(service.wire_name);
        let wire_name = go_string_literal(operation.wire_name);
        output.push_str(&format!(
            "\tcase input.Service == {service_name} && input.Operation == {wire_name}:\n\
             \trequest, ok := input.Input.({request})\n\
             \tif !ok {{\n\
             \t\treturn {failed}(ctx, fmt.Errorf(\"system Nexus operation %s/%s expects {request}, got %T\", {service_name}, {wire_name}, input.Input))\n\
             \t}}\n\
             \treturn outbound.{method}(ctx, &request)\n"
        ));
    }
    output.push_str(&format!(
        "\t}}\n\
         \treturn {failed}(ctx, fmt.Errorf(\"unsupported system Nexus operation %s/%s\", input.Service, input.Operation))\n\
         }}\n\n"
    ));
    output.push_str(&format!("type {interface} interface {{\n"));
    for (_, operation) in &operations {
        let method = go_field_name(operation.name);
        let request = &operation.input_type;
        output.push_str(&format!(
            "\t// {method} intercepts workflow.{method}.\n\
             \t{method}(ctx {context}, request *{request}) {future}\n"
        ));
    }
    output.push_str("}\n");

    for (service, operation) in &operations {
        let method = go_field_name(operation.name);
        let request = &operation.input_type;
        let service_name = go_string_literal(service.wire_name);
        let wire_name = go_string_literal(operation.wire_name);
        output.push_str(&format!(
            "\nfunc (w *{base}) {method}(ctx {context}, request *{request}) {future} {{\n\
             \treturn w.Next.{method}(ctx, request)\n\
             }}\n\
             \nfunc (wc *{terminal}) {method}(ctx {context}, request *{request}) {future} {{\n\
             \treturn wc.ExecuteNexusOperation(ctx, {execute_input}{{\n\
             \t\tClient: {new_client}({service_name}),\n\
             \t\tOperation: {wire_name},\n\
             \t\tInput: *request,\n\
             \t\tNexusHeader: nexus.Header{{}},\n\
             \t}})\n\
             }}\n"
        ));
    }
    if !sdk {
        output.push_str(&format!(
            "\n// --- Test-only adapters ---\n\
             \ntype {base} struct {{\n\
             \tNext {interface}\n\
             }}\n\
             \ntype {terminal} struct {{\n\
             \t{runtime}\n\
             }}\n\
             \ntype {dispatch_input} struct {{\n\
             \tService string\n\
             \tOperation string\n\
             \tInput any\n\
             }}\n\
             \ntype {runtime} interface {{\n\
             \tFailedFuture(ctx {context}, err error) {future}\n\
             \tExecuteNexusOperation(ctx {context}, input {execute_input}) {future}\n\
             }}\n"
        ));
    }
    Ok(output)
}
