package controller

import (
	"testing"

	"github.com/stretchr/testify/require"
)

func TestParseAssistantL1AutoReviewDecision(t *testing.T) {
	decision, err := parseAssistantL1AutoReviewDecision([]byte(`prefix {"decision":"approve","confidence":0.97,"note":"用途具体且合法"} suffix`))
	require.NoError(t, err)
	require.Equal(t, "approve", decision.Decision)
	require.InDelta(t, 0.97, decision.Confidence, 0.0001)
	require.Equal(t, "用途具体且合法", decision.Note)
}

func TestParseAssistantL1AutoReviewDecisionFailsClosed(t *testing.T) {
	for _, body := range []string{
		`{"decision":"approve","confidence":0.89,"note":"too uncertain"}`,
		`{"decision":"reject","confidence":0.99,"note":"unsupported decision"}`,
		`{"decision":"approve","confidence":1.2,"note":"invalid confidence"}`,
		`not json`,
	} {
		decision, err := parseAssistantL1AutoReviewDecision([]byte(body))
		if body == `{"decision":"approve","confidence":0.89,"note":"too uncertain"}` {
			require.NoError(t, err)
			require.Equal(t, "approve", decision.Decision)
			continue
		}
		require.Error(t, err, body)
	}
}
