package tests

import (
	"reflect"
)

type nexusCall struct {
	Operation string
	Input     any
}

func operationNames(calls []nexusCall) []string {
	names := make([]string, len(calls))
	for i, call := range calls {
		names[i] = call.Operation
	}
	return names
}

func stringField(value any, name string) string {
	result, _ := field(value, name).(string)
	return result
}

func stringSliceField(value any, name string) []string {
	switch values := field(value, name).(type) {
	case []string:
		return values
	case []any:
		result := make([]string, len(values))
		for i, value := range values {
			item, ok := value.(string)
			if !ok {
				return nil
			}
			result[i] = item
		}
		return result
	default:
		return nil
	}
}

func field(value any, name string) any {
	reflected := reflect.ValueOf(value)
	for reflected.IsValid() && (reflected.Kind() == reflect.Interface || reflected.Kind() == reflect.Pointer) {
		if reflected.IsNil() {
			return nil
		}
		reflected = reflected.Elem()
	}
	if !reflected.IsValid() || reflected.Kind() != reflect.Struct {
		if reflected.IsValid() && reflected.Kind() == reflect.Map && reflected.Type().Key().Kind() == reflect.String {
			mapValue := reflected.MapIndex(reflect.ValueOf(name))
			if mapValue.IsValid() {
				return mapValue.Interface()
			}
		}
		return nil
	}
	field := reflected.FieldByName(name)
	if !field.IsValid() || !field.CanInterface() {
		return nil
	}
	return field.Interface()
}

func mapValue(value any) map[string]any {
	if values, ok := value.(map[string]any); ok {
		return values
	}
	reflected := reflect.ValueOf(value)
	if !reflected.IsValid() || reflected.Kind() != reflect.Map || reflected.Type().Key().Kind() != reflect.String {
		return nil
	}
	values := make(map[string]any, reflected.Len())
	iter := reflected.MapRange()
	for iter.Next() {
		values[iter.Key().String()] = iter.Value().Interface()
	}
	return values
}
