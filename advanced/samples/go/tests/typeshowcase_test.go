package tests

import (
	"context"
	"reflect"
	"testing"

	"github.com/nexus-rpc/sdk-go/nexus"
	"github.com/stretchr/testify/suite"
	"go.temporal.io/sdk/testsuite"
	"go.temporal.io/sdk/workflow"

	"advanced/samples/go/typeshowcase"
)

const typeShowcaseServiceName = "TypeShowcase"

type TypeShowcaseIntegrationSuite struct {
	suite.Suite
	testsuite.WorkflowTestSuite
	env   *testsuite.TestWorkflowEnvironment
	calls []nexusCall
}

func (s *TypeShowcaseIntegrationSuite) SetupTest() {
	s.env = s.NewTestWorkflowEnvironment()
	s.calls = nil

	recordSync := nexus.NewSyncOperation("RecordSync",
		func(ctx context.Context, input any, opts nexus.StartOperationOptions) (nexus.NoValue, error) {
			s.calls = append(s.calls, nexusCall{"RecordSync", input})
			return nil, nil
		})
	user := func(input any) typeshowcase.User {
		// The workflow test environment serializes native WIT models through its
		// ordinary data converter. Keep the response scalar-only; the request
		// assertions below are where this suite exercises the rich native shapes.
		return typeshowcase.User{UserID: stringField(input, "UserID"), Email: "alice@example.com", DisplayName: "Alice"}
	}
	getUser := nexus.NewSyncOperation("GetUser",
		func(ctx context.Context, input any, opts nexus.StartOperationOptions) (typeshowcase.User, error) {
			s.calls = append(s.calls, nexusCall{"GetUser", input})
			return user(input), nil
		})
	updateEmail := nexus.NewSyncOperation("UpdateEmail",
		func(ctx context.Context, input any, opts nexus.StartOperationOptions) (typeshowcase.User, error) {
			s.calls = append(s.calls, nexusCall{"UpdateEmail", input})
			value := user(input)
			value.Email = stringField(input, "Email")
			return value, nil
		})
	rename := nexus.NewSyncOperation("Rename",
		func(ctx context.Context, input any, opts nexus.StartOperationOptions) (typeshowcase.User, error) {
			s.calls = append(s.calls, nexusCall{"Rename", input})
			value := user(input)
			value.DisplayName = stringField(input, "DisplayName")
			return value, nil
		})
	setProfile := nexus.NewSyncOperation("SetProfile",
		func(ctx context.Context, input any, opts nexus.StartOperationOptions) (typeshowcase.User, error) {
			s.calls = append(s.calls, nexusCall{"SetProfile", input})
			return user(input), nil
		})
	deactivate := nexus.NewSyncOperation("Deactivate",
		func(ctx context.Context, input any, opts nexus.StartOperationOptions) (nexus.NoValue, error) {
			s.calls = append(s.calls, nexusCall{"Deactivate", input})
			return nil, nil
		})

	service := nexus.NewService(typeShowcaseServiceName)
	s.NoError(service.Register(getUser, updateEmail, rename, setProfile, recordSync, deactivate))
	s.env.RegisterNexusService(service)
}

func sampleProfile() typeshowcase.UserProfile {
	return typeshowcase.UserProfile{
		Tags:               []string{"admin", "beta"},
		Metadata:           map[string]string{"tier": "enterprise"},
		Capabilities:       typeshowcase.UserCapabilityReadProfile | typeshowcase.UserCapabilityUpdateEmail,
		SyncState:          typeshowcase.Result[string, string]{Result: "synced"},
		NotificationTarget: typeshowcase.NotificationTargetEmail{Value: "alice@example.com"},
		Address: &typeshowcase.PostalAddress{
			Street: "1 Main St", City: "Portland", Country: "US",
			Coordinates: &typeshowcase.Tuple2[float64, float64]{First: 45.5152, Second: -122.6784},
		},
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

func TestTypeShowcaseIntegrationSuite(t *testing.T) {
	suite.Run(t, &TypeShowcaseIntegrationSuite{})
}

func sampleSyncReport() typeshowcase.SyncReport {
	return typeshowcase.SyncReport{
		Route: []typeshowcase.Tuple2[float64, float64]{
			{First: 45.5152, Second: -122.6784},
			{First: 47.6062, Second: -122.3321},
		},
		Attempts: []typeshowcase.Result[string, string]{
			{Result: "synced"},
			{Error: "timeout"},
		},
		RegionStatus: map[string]typeshowcase.Result[string, string]{
			"west":    {Result: "healthy"},
			"central": {Error: "degraded"},
		},
	}
}

func (s *TypeShowcaseIntegrationSuite) TestOperationsAndResourceMethods() {
	s.env.ExecuteWorkflow(func(ctx workflow.Context) error {
		var user typeshowcase.User
		if err := typeshowcase.GetUser(ctx, typeshowcase.GetUserOptions{UserID: "user-123", ConsistencyToken: "read-123"}).Get(ctx, &user); err != nil {
			return err
		}
		var updatedUser typeshowcase.User
		if err := typeshowcase.UpdateEmail(ctx, typeshowcase.UpdateEmailOptions{UserID: "user-123", Email: "new@example.com"}).Get(ctx, &updatedUser); err != nil {
			return err
		}
		var renamedUser typeshowcase.User
		if err := typeshowcase.Rename(ctx, typeshowcase.RenameOptions{UserID: "user-123", DisplayName: "New Name"}).Get(ctx, &renamedUser); err != nil {
			return err
		}
		var profileUser typeshowcase.User
		if err := typeshowcase.SetProfile(ctx, typeshowcase.SetProfileOptions{UserID: "user-123", Profile: sampleProfile()}).Get(ctx, &profileUser); err != nil {
			return err
		}
		if err := typeshowcase.Deactivate(ctx, typeshowcase.DeactivateOptions{UserID: "user-123", Reason: "requested"}).Get(ctx, nil); err != nil {
			return err
		}
		var resourceUpdatedUser typeshowcase.User
		if err := user.UpdateEmail(ctx, "resource@example.com").Get(ctx, &resourceUpdatedUser); err != nil {
			return err
		}
		var resourceRenamedUser typeshowcase.User
		if err := user.Rename(ctx, "Resource Name").Get(ctx, &resourceRenamedUser); err != nil {
			return err
		}
		if err := user.Deactivate(ctx, "resource-requested").Get(ctx, nil); err != nil {
			return err
		}
		// Container fields exercise the generated generic Tuple2 and Result
		// types as nested list and map values.
		return typeshowcase.RecordSync(ctx, typeshowcase.RecordSyncOptions{
			UserID: "user-123",
			Report: sampleSyncReport(),
		}).Get(ctx, nil)
	})

	s.True(s.env.IsWorkflowCompleted())
	s.NoError(s.env.GetWorkflowError())
	s.Require().Len(s.calls, 9)
	s.Equal([]string{"GetUser", "UpdateEmail", "Rename", "SetProfile", "Deactivate", "UpdateEmail", "Rename", "Deactivate", "RecordSync"}, operationNames(s.calls))
	s.Equal("read-123", stringField(s.calls[0].Input, "ConsistencyToken"))
	s.Equal("new@example.com", stringField(s.calls[1].Input, "Email"))
	s.Equal("New Name", stringField(s.calls[2].Input, "DisplayName"))
	profile := mapValue(field(s.calls[3].Input, "Profile"))
	s.NotNil(profile)
	s.Contains(profile, "Tags")
	s.Contains(profile, "Metadata")
	s.Contains(profile, "Capabilities")
	s.Contains(profile, "SyncState")
	s.Contains(profile, "NotificationTarget")
	s.Contains(profile, "Address")
	s.Equal("requested", stringField(s.calls[4].Input, "Reason"))
	s.Equal("user-123", stringField(s.calls[5].Input, "UserID"))
	s.Equal("resource@example.com", stringField(s.calls[5].Input, "Email"))
	s.Equal("Resource Name", stringField(s.calls[6].Input, "DisplayName"))
	s.Equal("resource-requested", stringField(s.calls[7].Input, "Reason"))
	s.Equal("user-123", stringField(s.calls[8].Input, "UserID"))
	report := mapValue(field(s.calls[8].Input, "Report"))
	s.NotNil(report)
	s.Contains(report, "Route")
	s.Contains(report, "Attempts")
	s.Contains(report, "RegionStatus")
}
