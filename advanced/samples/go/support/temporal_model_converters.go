package support

import (
	"time"

	common "go.temporal.io/api/common/v1"
	enums "go.temporal.io/api/enums/v1"
	failurepb "go.temporal.io/api/failure/v1"
	taskqueue "go.temporal.io/api/taskqueue/v1"
	"go.temporal.io/sdk/temporal"
	"go.temporal.io/sdk/workflow"
	"google.golang.org/protobuf/types/known/durationpb"
)

func WorkflowNamespace(ctx workflow.Context) string {
	return workflow.GetInfo(ctx).Namespace
}

func DurationToProto(d *time.Duration) (*durationpb.Duration, error) {
	if d == nil {
		return nil, nil
	}
	return durationpb.New(*d), nil
}

func DurationFromProto(d *durationpb.Duration) (*time.Duration, error) {
	if d == nil {
		return nil, nil
	}
	value := d.AsDuration()
	return &value, nil
}

func TaskQueueToProto(name *string) (*taskqueue.TaskQueue, error) {
	if name == nil {
		return nil, nil
	}
	return &taskqueue.TaskQueue{Name: *name, Kind: enums.TASK_QUEUE_KIND_NORMAL}, nil
}

func TaskQueueFromProto(tq *taskqueue.TaskQueue) (*string, error) {
	if tq == nil {
		return nil, nil
	}
	value := tq.GetName()
	return &value, nil
}

func WorkflowTypeToProto(name *string) (*common.WorkflowType, error) {
	if name == nil {
		return nil, nil
	}
	return &common.WorkflowType{Name: *name}, nil
}

func WorkflowTypeFromProto(t *common.WorkflowType) (*string, error) {
	if t == nil {
		return nil, nil
	}
	value := t.GetName()
	return &value, nil
}

func RetryPolicyToProto(p *temporal.RetryPolicy) (*common.RetryPolicy, error) {
	if p == nil {
		return nil, nil
	}
	return &common.RetryPolicy{
		InitialInterval:        durationpb.New(p.InitialInterval),
		BackoffCoefficient:     p.BackoffCoefficient,
		MaximumInterval:        durationpb.New(p.MaximumInterval),
		MaximumAttempts:        p.MaximumAttempts,
		NonRetryableErrorTypes: p.NonRetryableErrorTypes,
	}, nil
}

func RetryPolicyFromProto(p *common.RetryPolicy) (*temporal.RetryPolicy, error) {
	if p == nil {
		return nil, nil
	}
	policy := temporal.RetryPolicy{
		BackoffCoefficient:     p.GetBackoffCoefficient(),
		MaximumAttempts:        p.GetMaximumAttempts(),
		NonRetryableErrorTypes: p.GetNonRetryableErrorTypes(),
	}
	if interval := p.GetInitialInterval(); interval != nil {
		policy.InitialInterval = interval.AsDuration()
	}
	if interval := p.GetMaximumInterval(); interval != nil {
		policy.MaximumInterval = interval.AsDuration()
	}
	return &policy, nil
}

func PriorityToProto(p *temporal.Priority) (*common.Priority, error) {
	if p == nil || *p == (temporal.Priority{}) {
		return nil, nil
	}
	return &common.Priority{
		PriorityKey:    int32(p.PriorityKey),
		FairnessKey:    p.FairnessKey,
		FairnessWeight: p.FairnessWeight,
	}, nil
}

func PriorityFromProto(p *common.Priority) (*temporal.Priority, error) {
	if p == nil {
		return nil, nil
	}
	return &temporal.Priority{
		PriorityKey:    int(p.GetPriorityKey()),
		FairnessKey:    p.GetFairnessKey(),
		FairnessWeight: p.GetFairnessWeight(),
	}, nil
}

// Ordinary transfer callbacks have no caller-supplied converter. This sample
// uses the default failure converter's fixed payload encoding for failure details.
func FailureToProto(value error) (*failurepb.Failure, error) {
	return temporal.NewDefaultFailureConverter(temporal.DefaultFailureConverterOptions{}).ErrorToFailure(value), nil
}

func FailureFromProto(failure *failurepb.Failure) (error, error) {
	if failure == nil {
		return nil, nil
	}
	return temporal.NewDefaultFailureConverter(temporal.DefaultFailureConverterOptions{}).FailureToError(failure), nil
}
