package handles

import "errors"

// FunctionResult combines the request's function name with its operation result.
type FunctionResult struct {
	FunctionName string
	Value        string
}

func NewFunctionResult(functionName, value string) (FunctionResult, error) {
	if value == "" {
		return FunctionResult{}, errors.New("function result is empty")
	}
	return FunctionResult{FunctionName: functionName, Value: value}, nil
}
