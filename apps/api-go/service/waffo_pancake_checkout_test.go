package service

import (
	"encoding/json"
	"testing"

	"github.com/stretchr/testify/require"
	pancake "github.com/waffo-com/waffo-pancake-sdk-go"
)

func TestBuildWaffoPancakeSDKCheckoutParamsRegionMapping(t *testing.T) {
	testCases := []struct {
		name            string
		region          string
		language        string
		expectsBilling  bool
		expectedCountry string
	}{
		{name: "explicit china", region: "china", language: "en", expectsBilling: true, expectedCountry: "CN"},
		{name: "explicit global overrides chinese language", region: "global", language: "zh-Hans", expectsBilling: false},
		{name: "empty region infers china from simplified chinese", language: "zh-Hans", expectsBilling: true, expectedCountry: "CN"},
		{name: "empty region infers china from traditional chinese", language: "zh-Hant-TW", expectsBilling: true, expectedCountry: "CN"},
		{name: "empty region infers china from Hong Kong traditional chinese", language: "zh-Hant-HK", expectsBilling: true, expectedCountry: "CN"},
		{name: "empty region infers china from Hong Kong simplified chinese", language: "zh-Hans-HK", expectsBilling: true, expectedCountry: "CN"},
		{name: "empty region defaults global for other language", language: "en", expectsBilling: false},
		{name: "empty region defaults global without language", expectsBilling: false},
		{name: "invalid region cannot select china", region: "CN", language: "zh-Hans", expectsBilling: false},
	}

	for _, tc := range testCases {
		t.Run(tc.name, func(t *testing.T) {
			sdkParams, err := buildWaffoPancakeSDKCheckoutParams(&WaffoPancakeCreateSessionParams{
				ProductID:        "PROD_test",
				BuyerIdentity:    "new-api-user-1",
				CheckoutRegion:   tc.region,
				CheckoutLanguage: tc.language,
			})
			require.NoError(t, err)

			if !tc.expectsBilling {
				require.Nil(t, sdkParams.BillingDetail)
				body, marshalErr := json.Marshal(sdkParams.CreateCheckoutSessionParams)
				require.NoError(t, marshalErr)
				require.NotContains(t, string(body), "billingDetail")
				return
			}

			require.NotNil(t, sdkParams.BillingDetail)
			require.Equal(t, tc.expectedCountry, sdkParams.BillingDetail.Country)
			require.False(t, sdkParams.BillingDetail.IsBusiness)
			require.Nil(t, sdkParams.BillingDetail.Postcode)
			require.Nil(t, sdkParams.BillingDetail.State)
			require.Nil(t, sdkParams.BillingDetail.BusinessName)
			require.Nil(t, sdkParams.BillingDetail.TaxID)
		})
	}
}

func TestBuildWaffoPancakeSDKCheckoutParamsLanguageAllowList(t *testing.T) {
	valid, err := buildWaffoPancakeSDKCheckoutParams(&WaffoPancakeCreateSessionParams{
		ProductID:        "PROD_test",
		BuyerIdentity:    "new-api-user-1",
		CheckoutLanguage: "zh-Hans",
	})
	require.NoError(t, err)
	require.NotNil(t, valid.Language)
	require.Equal(t, pancake.CashierLanguage("zh-Hans"), *valid.Language)

	invalid, err := buildWaffoPancakeSDKCheckoutParams(&WaffoPancakeCreateSessionParams{
		ProductID:        "PROD_test",
		BuyerIdentity:    "new-api-user-1",
		CheckoutLanguage: "zh-CN",
	})
	require.NoError(t, err)
	require.Nil(t, invalid.Language)
}

func TestBuildWaffoPancakeSDKCheckoutParamsPreservesOrderMetadata(t *testing.T) {
	sdkParams, err := buildWaffoPancakeSDKCheckoutParams(&WaffoPancakeCreateSessionParams{
		ProductID:     "PROD_test",
		BuyerIdentity: "new-api-user-1",
		OrderMetadata: map[string]string{"lmm_product_id": "PROD_test", "lmm_plan_id": "42"},
	})
	require.NoError(t, err)
	require.Equal(t, map[string]string{"lmm_product_id": "PROD_test", "lmm_plan_id": "42"}, sdkParams.Metadata)
}

func TestResolveWaffoPancakeCheckoutRegionCannotSelectArbitraryCountry(t *testing.T) {
	for _, value := range []string{"US", "CN", "DE", "china;country=US"} {
		require.Equal(t, WaffoPancakeCheckoutRegionGlobal, ResolveWaffoPancakeCheckoutRegion(value, "zh-Hans"))
	}
}
