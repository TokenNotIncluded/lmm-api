package controller

import (
	"testing"

	"github.com/stretchr/testify/require"
)

func TestCreemWebhookReceiptLogOmitsSensitiveValues(t *testing.T) {
	logLine := creemWebhookReceiptLog("/api/creem/webhook", "198.51.100.7", len(`{"customer":{"email":"buyer@example.com"},"secret":"payload"}`))

	require.Contains(t, logLine, "body_bytes=")
	require.NotContains(t, logLine, "buyer@example.com")
	require.NotContains(t, logLine, "secret")
	require.NotContains(t, logLine, "payload")
	require.NotContains(t, logLine, "signature")
}

func TestCreemPaymentLogsUseBoundedSummaries(t *testing.T) {
	requestLog := creemPaymentRequestLog(42, len(`{"email":"buyer@example.com","secret":"payload"}`))
	responseLog := creemAPIResponseLog("creem-trade-1", 200, len(`{"checkout_url":"https://pay.example/secret-token"}`))

	for _, logLine := range []string{requestLog, responseLog} {
		require.Contains(t, logLine, "body_bytes=")
		require.NotContains(t, logLine, "buyer@example.com")
		require.NotContains(t, logLine, "secret")
		require.NotContains(t, logLine, "payload")
		require.NotContains(t, logLine, "checkout_url")
		require.NotContains(t, logLine, "secret-token")
	}
}
