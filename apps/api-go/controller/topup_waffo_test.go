package controller

import (
	"testing"

	"github.com/stretchr/testify/require"
)

func TestWaffoWebhookReceiptLogOmitsSensitiveValues(t *testing.T) {
	logLine := waffoWebhookReceiptLog("/api/waffo/webhook", "198.51.100.7", len(`{"secret":"payload"}`))

	require.Contains(t, logLine, "body_bytes=")
	require.NotContains(t, logLine, "secret")
	require.NotContains(t, logLine, "payload")
	require.NotContains(t, logLine, "signature")
}
