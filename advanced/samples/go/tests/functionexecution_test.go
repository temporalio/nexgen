package tests

import (
	"context"
	"reflect"
	"testing"

	"github.com/nexus-rpc/sdk-go/nexus"
	"github.com/stretchr/testify/suite"
	"go.temporal.io/sdk/testsuite"
	"go.temporal.io/sdk/workflow"

	"go.temporal.io/sdk/advanced/samples/go/functionexecution"
)

const functionExecutionServiceName = "FunctionExecution"

func validFunction(name string, enabled bool) string {
	return name
}
func validCountedFunction(name string, count int32) string {
	return name
}

func validVarargsFunction(args ...string) string {
	if len(args) == 0 {
		return ""
	}
	return args[0]
}

type FunctionExecutionIntegrationSuite struct {
	suite.Suite
	testsuite.WorkflowTestSuite
	env   *testsuite.TestWorkflowEnvironment
	calls []nexusCall
}

func (s *FunctionExecutionIntegrationSuite) SetupTest() {
	s.env = s.NewTestWorkflowEnvironment()
	s.calls = nil

	executeFunction := nexus.NewSyncOperation("ExecuteFunction",
		func(ctx context.Context, input any, opts nexus.StartOperationOptions) (functionexecution.ExecuteFunctionResult, error) {
			s.calls = append(s.calls, nexusCall{"ExecuteFunction", input})
			return functionexecution.ExecuteFunctionResult{Value: "executed"}, nil
		})

	executeCountedFunction := nexus.NewSyncOperation("ExecuteCountedFunction",
		func(ctx context.Context, input any, opts nexus.StartOperationOptions) (functionexecution.ExecuteCountedFunctionResult, error) {
			s.calls = append(s.calls, nexusCall{"ExecuteCountedFunction", input})
			return functionexecution.ExecuteCountedFunctionResult{Value: "counted"}, nil
		})

	executeNamedFunction := nexus.NewSyncOperation("ExecuteNamedFunction",
		func(ctx context.Context, input any, opts nexus.StartOperationOptions) (functionexecution.ExecuteNamedFunctionResult, error) {
			s.calls = append(s.calls, nexusCall{"ExecuteNamedFunction", input})
			return functionexecution.ExecuteNamedFunctionResult{Value: "named"}, nil
		})

	executeVarargsFunction := nexus.NewSyncOperation("ExecuteVarargsFunction",
		func(ctx context.Context, input any, opts nexus.StartOperationOptions) (functionexecution.ExecuteVarargsFunctionResult, error) {
			s.calls = append(s.calls, nexusCall{"ExecuteVarargsFunction", input})
			return functionexecution.ExecuteVarargsFunctionResult{Value: "varargs"}, nil
		})

	executeNamedVarargsFunction := nexus.NewSyncOperation("ExecuteNamedVarargsFunction",
		func(ctx context.Context, input any, opts nexus.StartOperationOptions) (functionexecution.ExecuteNamedVarargsFunctionResult, error) {
			s.calls = append(s.calls, nexusCall{"ExecuteNamedVarargsFunction", input})
			return functionexecution.ExecuteNamedVarargsFunctionResult{Value: "named-varargs"}, nil
		})

	service := nexus.NewService(functionExecutionServiceName)
	s.NoError(service.Register(
		executeFunction,
		executeCountedFunction,
		executeNamedFunction,
		executeVarargsFunction,
		executeNamedVarargsFunction,
	))
	s.env.RegisterNexusService(service)
}

func TestFunctionExecutionIntegrationSuite(t *testing.T) {
	suite.Run(t, &FunctionExecutionIntegrationSuite{})
}

func (s *FunctionExecutionIntegrationSuite) TestFunctionArgumentForms() {
	s.env.ExecuteWorkflow(func(ctx workflow.Context) ([]string, error) {
		values := make([]string, 0, 6)

		// Invoke Public APIs
		var direct functionexecution.ExecuteFunctionResult
		if err := functionexecution.ExecuteFunction(
			ctx, functionexecution.ExecuteFunctionOptions{}, validFunction, "one", true,
		).Get(ctx, &direct); err != nil {
			return nil, err
		}
		values = append(values, direct.Value)

		var counted functionexecution.ExecuteCountedFunctionResult
		if err := functionexecution.ExecuteCountedFunction(
			ctx, functionexecution.ExecuteCountedFunctionOptions{}, validCountedFunction, "one", 7,
		).Get(ctx, &counted); err != nil {
			return nil, err
		}
		values = append(values, counted.Value)

		var named functionexecution.ExecuteNamedFunctionResult
		if err := functionexecution.ExecuteNamedFunction(
			ctx, functionexecution.ExecuteNamedFunctionOptions{}, validFunction, "one", true,
		).Get(ctx, &named); err != nil {
			return nil, err
		}
		values = append(values, named.Value)

		var varargs functionexecution.ExecuteVarargsFunctionResult
		if err := functionexecution.ExecuteVarargsFunction(
			ctx, functionexecution.ExecuteVarargsFunctionOptions{}, validVarargsFunction, "one", "two",
		).Get(ctx, &varargs); err != nil {
			return nil, err
		}
		values = append(values, varargs.Value)

		var namedVarargs functionexecution.ExecuteNamedVarargsFunctionResult
		if err := functionexecution.ExecuteNamedVarargsFunction(
			ctx, functionexecution.ExecuteNamedVarargsFunctionOptions{}, "named-varargs-function", "one", "two",
		).Get(ctx, &namedVarargs); err != nil {
			return nil, err
		}
		values = append(values, namedVarargs.Value)

		var functionVarargs functionexecution.ExecuteNamedVarargsFunctionResult
		if err := functionexecution.ExecuteNamedVarargsFunction(
			ctx, functionexecution.ExecuteNamedVarargsFunctionOptions{}, validVarargsFunction, "one", "two",
		).Get(ctx, &functionVarargs); err != nil {
			return nil, err
		}
		return append(values, functionVarargs.Value), nil
	})

	s.True(s.env.IsWorkflowCompleted())
	s.NoError(s.env.GetWorkflowError())
	var values []string
	s.NoError(s.env.GetWorkflowResult(&values))
	s.Equal([]string{"executed", "counted", "named", "varargs", "named-varargs", "named-varargs"}, values)

	s.Require().Len(s.calls, 6)
	s.Equal([]string{
		"ExecuteFunction",
		"ExecuteCountedFunction",
		"ExecuteNamedFunction",
		"ExecuteVarargsFunction",
		"ExecuteNamedVarargsFunction",
		"ExecuteNamedVarargsFunction",
	}, operationNames(s.calls))
	s.Equal("validVarargsFunction", stringField(s.calls[3].Input, "Function"))
	s.Equal([]string{"one", "two"}, stringSliceField(s.calls[3].Input, "Args"))
	s.Equal("named-varargs-function", stringField(s.calls[4].Input, "Function"))
	s.Equal("validVarargsFunction", stringField(s.calls[5].Input, "Function"))
}

func stringSliceField(value any, name string) []string {
	reflected := reflect.ValueOf(value)
	if reflected.Kind() == reflect.Pointer {
		reflected = reflected.Elem()
	}
	if reflected.Kind() == reflect.Struct {
		field := reflected.FieldByName(name)
		if values := stringSliceValue(field); values != nil {
			return values
		}
	}
	if reflected.Kind() == reflect.Map && reflected.Type().Key().Kind() == reflect.String {
		mapValue := reflected.MapIndex(reflect.ValueOf(name))
		if values := stringSliceValue(mapValue); values != nil {
			return values
		}
	}
	return nil
}

func stringSliceValue(value reflect.Value) []string {
	if !value.IsValid() {
		return nil
	}
	for value.Kind() == reflect.Interface || value.Kind() == reflect.Pointer {
		if value.IsNil() {
			return nil
		}
		value = value.Elem()
	}
	if value.Kind() != reflect.Slice {
		return nil
	}
	values := make([]string, 0, value.Len())
	for i := 0; i < value.Len(); i++ {
		item := value.Index(i)
		for item.Kind() == reflect.Interface || item.Kind() == reflect.Pointer {
			if item.IsNil() {
				item = reflect.Value{}
				break
			}
			item = item.Elem()
		}
		if !item.IsValid() {
			values = append(values, "")
			continue
		}
		if item.Kind() != reflect.String {
			return nil
		}
		values = append(values, item.String())
	}
	return values
}
