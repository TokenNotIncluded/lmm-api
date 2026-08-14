package model

import (
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/constant"
	"github.com/stretchr/testify/require"
)

func TestTaskQueryLimitIsAlwaysBounded(t *testing.T) {
	tests := []struct {
		name  string
		input int
		want  int
	}{
		{name: "negative uses safe default", input: -1, want: constant.DefaultTaskQueryLimit},
		{name: "zero uses safe default", input: 0, want: constant.DefaultTaskQueryLimit},
		{name: "normal value preserved", input: 25, want: 25},
		{name: "oversized value capped", input: constant.MaxTaskQueryLimit + 1, want: constant.MaxTaskQueryLimit},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			require.Equal(t, test.want, normalizeTaskQueryLimit(test.input))
		})
	}
}
