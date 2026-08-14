package service

import (
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/relaykit/dto"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestApplyResponsesUsageNormalizesTokensWithoutTrustingBillingMetadata(t *testing.T) {
	localBilling := &dto.BillingUsage{Source: "local", Semantic: "local"}
	src := &dto.Usage{
		InputTokens:  11,
		OutputTokens: 7,
		TotalTokens:  18,
		InputTokensDetails: &dto.InputTokenDetails{
			CachedTokens:         3,
			CachedCreationTokens: 2,
			CacheWriteTokens:     4,
		},
		OutputTokensDetails:  &dto.OutputTokenDetails{ReasoningTokens: 5, TextTokens: 2},
		PromptCacheHitTokens: 3,
		UsageSemantic:        "openai",
		UsageSource:          "upstream",
		BillingUsage: &dto.BillingUsage{
			Source:   "hostile-upstream",
			Semantic: "hostile-upstream",
			OpenAIUsage: &dto.Usage{
				PromptTokens: 999999,
			},
		},
	}
	dst := &dto.Usage{UsageSemantic: "local", UsageSource: "local", BillingUsage: localBilling}
	ApplyResponsesUsage(dst, src)

	assert.Equal(t, 11, dst.PromptTokens)
	assert.Equal(t, 7, dst.CompletionTokens)
	assert.Equal(t, 18, dst.TotalTokens)
	require.NotNil(t, dst.InputTokensDetails)
	assert.Equal(t, 4, dst.PromptTokensDetails.CacheWriteTokens)
	assert.Equal(t, 3, dst.PromptTokensDetails.CachedTokens)
	require.NotNil(t, dst.OutputTokensDetails)
	assert.Equal(t, 5, dst.CompletionTokenDetails.ReasoningTokens)
	assert.Equal(t, 5, dst.OutputTokensDetails.ReasoningTokens)
	assert.Equal(t, "local", dst.UsageSemantic)
	assert.Equal(t, "local", dst.UsageSource)
	assert.Same(t, localBilling, dst.BillingUsage)

	src.InputTokensDetails.CachedTokens = 99
	src.OutputTokensDetails.ReasoningTokens = 99
	assert.Equal(t, 3, dst.InputTokensDetails.CachedTokens)
	assert.Equal(t, 5, dst.OutputTokensDetails.ReasoningTokens)
}

func TestApplyResponsesUsageFallsBackToCompletionDetails(t *testing.T) {
	dst := &dto.Usage{}
	ApplyResponsesUsage(dst, &dto.Usage{CompletionTokenDetails: dto.OutputTokenDetails{ReasoningTokens: 9}})
	require.NotNil(t, dst.OutputTokensDetails)
	assert.Equal(t, 9, dst.OutputTokensDetails.ReasoningTokens)
}
