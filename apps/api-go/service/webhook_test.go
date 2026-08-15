package service

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/relaykit/dto"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestMarshalWebhookPayloadBoundsRequestBody(t *testing.T) {
	_, err := marshalWebhookPayload(dto.Notify{Content: strings.Repeat("x", webhookPayloadMaxBytes)})
	require.ErrorIs(t, err, common.ErrLimitExceeded)
}

func TestMarshalWebhookPayloadIncludesNormalNotificationFields(t *testing.T) {
	payload, err := marshalWebhookPayload(dto.Notify{
		Type:    "quota_exceed",
		Title:   "title",
		Content: "content %s",
		Values:  []interface{}{"value"},
	})
	require.NoError(t, err)
	assert.LessOrEqual(t, len(payload), webhookPayloadMaxBytes)

	var decoded WebhookPayload
	require.NoError(t, json.Unmarshal(payload, &decoded))
	assert.Equal(t, "quota_exceed", decoded.Type)
	assert.Equal(t, "title", decoded.Title)
	assert.Equal(t, "content value", decoded.Content)
	assert.Greater(t, decoded.Timestamp, int64(0))
}
