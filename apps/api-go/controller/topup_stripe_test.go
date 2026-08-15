package controller

import (
	"testing"

	"github.com/stretchr/testify/require"
)

func TestStripeWebhookReceiptLogOmitsSensitiveValues(t *testing.T) {
	logLine := stripeWebhookReceiptLog("/api/stripe/webhook", "198.51.100.7", len(`{"secret":"payload"}`))

	require.Contains(t, logLine, "body_bytes=")
	require.NotContains(t, logLine, "secret")
	require.NotContains(t, logLine, "payload")
	require.NotContains(t, logLine, "signature")
}
