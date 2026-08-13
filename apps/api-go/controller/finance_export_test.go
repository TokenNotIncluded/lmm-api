package controller

import (
	"archive/zip"
	"bytes"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/require"
)

func financeExportTestContext(query string) *gin.Context {
	gin.SetMode(gin.TestMode)
	req := httptest.NewRequest("GET", "/api/finance/export"+query, nil)
	c, _ := gin.CreateTestContext(httptest.NewRecorder())
	c.Request = req
	return c
}

func TestParseFinanceExportWindowDefaultsAndLimits(t *testing.T) {
	start, end, err := parseFinanceExportWindow(financeExportTestContext(""))
	require.NoError(t, err)
	require.InDelta(t, financeExportDefaultWindow, end-start, 2)

	_, _, err = parseFinanceExportWindow(financeExportTestContext("?start_timestamp=20&end_timestamp=10"))
	require.ErrorContains(t, err, "start_timestamp must be before end_timestamp")

	_, _, err = parseFinanceExportWindow(financeExportTestContext("?start_timestamp=1&end_timestamp=31536002"))
	require.ErrorContains(t, err, "cannot exceed")
}

func TestParseFinanceExportWindowAcceptsExactOneYearRange(t *testing.T) {
	start, end, err := parseFinanceExportWindow(financeExportTestContext("?start_timestamp=1700000000&end_timestamp=1731536000"))
	require.NoError(t, err)
	require.EqualValues(t, 1700000000, start)
	require.EqualValues(t, 1731536000, end)
}

func TestFinanceExportFilesAreRedactedAndClipboardFriendly(t *testing.T) {
	bundle := financeExportBundle{
		Manifest: financeExportManifest{
			SchemaVersion: "test",
			Rows:          map[string]int{"users": 1},
			Truncated:     map[string]bool{},
		},
		Options: map[string]string{
			"ModelPrice":                `{"gpt-test": 1.25}`,
			"ModelRatio":                `{"gpt-test": 2}`,
			"GroupRatio":                `{"default": 0.8}`,
			"TopupGroupRatio":           `{"default": 1.1}`,
			"tool_price_setting.prices": `{"web_search": 3}`,
		},
		Users:  []financeUserExport{{ID: 7, Username: "admin", Group: "default", Quota: 100, UsedQuota: 25}},
		TopUps: []financeTopUpExport{{ID: 1, Amount: 10, PaymentProvider: "stripe"}},
		Usage:  []financeUsageExport{{ID: 2, UserID: 7, Quota: 5}},
	}

	files, err := financeExportFiles(bundle)
	require.NoError(t, err)
	applyFinanceUserRatios(bundle.Users, bundle.Options)
	users, err := jsonFinanceFile(bundle.Users)
	require.NoError(t, err)
	require.Contains(t, string(users), `"effective_group_ratio": 0.8`)
	require.Contains(t, string(users), `"effective_topup_group_ratio": 1.1`)
	for _, name := range []string{
		"manifest.json",
		"financial-options.json",
		"model-prices-and-ratios.json",
		"effective-model-pricing.json",
		"users-balances.json",
		"channels-pricing.json",
		"subscription-plans.json",
		"topups.json",
		"subscription-orders.json",
		"usage-billing-records.json",
		"bounty-ledger.json",
		"checkins.json",
		"redemptions.json",
		"user-subscriptions.json",
	} {
		require.Contains(t, files, name)
	}
	text := string(financeExportText(files))
	require.Contains(t, text, "users-balances.json")
	require.Contains(t, text, `"gpt-test": 1.25`)
	require.NotContains(t, text, "provider_event_id")
	require.NotContains(t, text, "provider_payload")
}

func TestDecodeFinanceOptionPreservesNonJSONValues(t *testing.T) {
	require.Equal(t, "not-json", decodeFinanceOption("not-json"))
	require.Equal(t, map[string]any{"model": float64(2)}, decodeFinanceOption(`{"model":2}`))
}

func TestApplyFinanceUserRatiosUsesTheUserGroup(t *testing.T) {
	users := []financeUserExport{
		{Group: "default"},
		{Group: "vip"},
		{Group: "missing"},
	}
	applyFinanceUserRatios(users, map[string]string{
		"GroupRatio":      `{"default":1,"vip":0.8}`,
		"TopupGroupRatio": `{"default":1.1,"vip":0.9}`,
	})
	require.NotNil(t, users[0].GroupRatio)
	require.Equal(t, 1.0, *users[0].GroupRatio)
	require.Equal(t, 1.1, *users[0].TopupGroupRatio)
	require.Equal(t, 0.8, *users[1].GroupRatio)
	require.Equal(t, 0.9, *users[1].TopupGroupRatio)
	require.Nil(t, users[2].GroupRatio)
	require.Nil(t, users[2].TopupGroupRatio)
}

func TestSanitizeFinanceBaseURLRemovesCredentialsAndQuery(t *testing.T) {
	raw := "https://user:secret@example.com/v1?token=should-not-export"
	sanitized := sanitizeFinanceBaseURL(&raw)
	require.NotNil(t, sanitized)
	require.Equal(t, "https://example.com", *sanitized)
	require.Nil(t, sanitizeFinanceBaseURL(nil))
}

func TestFinanceExportTextHasStableSectionOrder(t *testing.T) {
	files := map[string][]byte{
		"manifest.json":                []byte("manifest"),
		"financial-options.json":       []byte("options"),
		"model-prices-and-ratios.json": []byte("prices"),
		"effective-model-pricing.json": []byte("effective-prices"),
		"users-balances.json":          []byte("users"),
		"channels-pricing.json":        []byte("channels"),
		"subscription-plans.json":      []byte("plans"),
		"topups.json":                  []byte("topups"),
		"subscription-orders.json":     []byte("orders"),
		"usage-billing-records.json":   []byte("usage"),
		"bounty-ledger.json":           []byte("ledger"),
		"checkins.json":                []byte("checkins"),
		"redemptions.json":             []byte("redemptions"),
		"user-subscriptions.json":      []byte("subscriptions"),
	}
	text := string(financeExportText(files))
	require.True(t, strings.Index(text, "manifest.json") < strings.Index(text, "users-balances.json"))
	require.True(t, strings.Index(text, "users-balances.json") < strings.Index(text, "usage-billing-records.json"))
}

func TestWriteFinanceZipContainsStableFiles(t *testing.T) {
	files := map[string][]byte{
		"manifest.json":                []byte("manifest"),
		"financial-options.json":       []byte("options"),
		"model-prices-and-ratios.json": []byte("prices"),
		"effective-model-pricing.json": []byte("effective-prices"),
		"users-balances.json":          []byte("users"),
		"channels-pricing.json":        []byte("channels"),
		"subscription-plans.json":      []byte("plans"),
		"topups.json":                  []byte("topups"),
		"subscription-orders.json":     []byte("orders"),
		"usage-billing-records.json":   []byte("usage"),
		"bounty-ledger.json":           []byte("ledger"),
		"checkins.json":                []byte("checkins"),
		"redemptions.json":             []byte("redemptions"),
		"user-subscriptions.json":      []byte("subscriptions"),
	}
	recorder := httptest.NewRecorder()
	context, _ := gin.CreateTestContext(recorder)
	require.NoError(t, writeFinanceZip(context, files))
	require.Equal(t, "application/zip", recorder.Header().Get("Content-Type"))
	archive, err := zip.NewReader(bytes.NewReader(recorder.Body.Bytes()), int64(recorder.Body.Len()))
	require.NoError(t, err)
	require.Len(t, archive.File, len(files))
	require.Equal(t, "manifest.json", archive.File[0].Name)
	require.Equal(t, "user-subscriptions.json", archive.File[len(archive.File)-1].Name)
}
