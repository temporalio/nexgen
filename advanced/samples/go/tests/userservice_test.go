package tests

import (
	"context"
	"testing"

	"github.com/nexus-rpc/sdk-go/nexus"
	"github.com/stretchr/testify/suite"
	"go.temporal.io/sdk/testsuite"
	"go.temporal.io/sdk/workflow"

	"advanced/samples/go/userservice"
)

const userServiceName = "UserService"

type UserServiceIntegrationSuite struct {
	suite.Suite
	testsuite.WorkflowTestSuite
	env   *testsuite.TestWorkflowEnvironment
	calls []nexusCall
}

func (s *UserServiceIntegrationSuite) SetupTest() {
	s.env = s.NewTestWorkflowEnvironment()
	s.calls = nil

	getUser := nexus.NewSyncOperation("GetUser",
		func(ctx context.Context, input any, opts nexus.StartOperationOptions) (userservice.User, error) {
			s.calls = append(s.calls, nexusCall{"GetUser", input})
			userID := stringField(input, "UserID")
			if userID == "missing" {
				return userservice.User{}, nexus.NewOperationFailedError("user not found")
			}
			return *userservice.NewUser(userID, "alice@example.com"), nil
		})

	updateEmail := nexus.NewSyncOperation("UpdateEmail",
		func(ctx context.Context, input any, opts nexus.StartOperationOptions) (userservice.User, error) {
			s.calls = append(s.calls, nexusCall{"UpdateEmail", input})
			return *userservice.NewUser(stringField(input, "UserID"), stringField(input, "Email")), nil
		})

	service := nexus.NewService(userServiceName)
	s.NoError(service.Register(getUser, updateEmail))
	s.env.RegisterNexusService(service)
}

func TestUserServiceIntegrationSuite(t *testing.T) {
	suite.Run(t, &UserServiceIntegrationSuite{})
}

func (s *UserServiceIntegrationSuite) TestOperationsAndResourceMethod() {
	s.env.ExecuteWorkflow(func(ctx workflow.Context) (*userservice.User, error) {
		var user userservice.User
		if err := userservice.GetUser(ctx, userservice.GetUserOptions{UserID: "user-123"}).Get(ctx, &user); err != nil {
			return nil, err
		}

		var updated userservice.User
		if err := userservice.UpdateEmail(ctx, userservice.UpdateEmailOptions{
			UserID: "user-123",
			Email:  "direct@example.com",
		}).Get(ctx, &updated); err != nil {
			return nil, err
		}

		// A WIT resource retains its identity fields. The generated receiver method
		// supplies UserID from user, leaving only the new email at the call site.
		var resourceUpdated userservice.User
		return &resourceUpdated, user.UpdateEmail(ctx, "resource@example.com").Get(ctx, &resourceUpdated)
	})

	s.True(s.env.IsWorkflowCompleted())
	s.NoError(s.env.GetWorkflowError())
	var result userservice.User
	s.NoError(s.env.GetWorkflowResult(&result))
	s.Equal(*userservice.NewUser("user-123", "resource@example.com"), result)

	s.Require().Len(s.calls, 3)
	s.Equal([]string{"GetUser", "UpdateEmail", "UpdateEmail"}, operationNames(s.calls))
	s.Equal("direct@example.com", stringField(s.calls[1].Input, "Email"))
	s.Equal("user-123", stringField(s.calls[2].Input, "UserID"))
	s.Equal("resource@example.com", stringField(s.calls[2].Input, "Email"))
}

func (s *UserServiceIntegrationSuite) TestOperationFailure() {
	s.env.ExecuteWorkflow(func(ctx workflow.Context) error {
		var result userservice.User
		return userservice.GetUser(ctx, userservice.GetUserOptions{UserID: "missing"}).Get(ctx, &result)
	})

	s.True(s.env.IsWorkflowCompleted())
	s.Error(s.env.GetWorkflowError())
	s.Require().Len(s.calls, 1)
	s.Equal("GetUser", s.calls[0].Operation)
}
