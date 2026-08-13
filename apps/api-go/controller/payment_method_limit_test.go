/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
package controller

import (
	"net/http/httptest"
	"testing"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/setting/operation_setting"
	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestConfiguredPaymentMethodMaxTopUpUsesMostRestrictiveDuplicate(t *testing.T) {
	withPaymentMethods(t, []map[string]string{
		{"name": "LinuxDO A", "type": "epay", "max_topup": "20"},
		{"name": "LinuxDO B", "type": "epay", "max_topup": "7.5"},
	})

	limit, configured, err := configuredPaymentMethodMaxTopUp("epay")
	require.NoError(t, err)
	assert.True(t, configured)
	assert.Equal(t, "7.5", limit.String())
}

func TestConfiguredPaymentMethodMaxTopUpRejectsInvalidLimit(t *testing.T) {
	withPaymentMethods(t, []map[string]string{{
		"name": "LinuxDO", "type": "epay", "max_topup": "0",
	}})

	_, configured, err := configuredPaymentMethodMaxTopUp("epay")
	assert.True(t, configured)
	assert.Error(t, err)
}

func TestConfiguredPaymentMethodMinTopUpUsesStrictestDuplicate(t *testing.T) {
	withPaymentMethods(t, []map[string]string{
		{"name": "Card A", "type": "epay", "min_topup": "2"},
		{"name": "Card B", "type": "epay", "min_topup": "7.5"},
	})

	minimum, configured, err := configuredPaymentMethodMinTopUp("epay")
	require.NoError(t, err)
	assert.True(t, configured)
	assert.Equal(t, "7.5", minimum.String())
}

func TestRequirePaymentMethodTopUpWithinLimitEnforcesMinimum(t *testing.T) {
	gin.SetMode(gin.TestMode)
	withPaymentMethods(t, []map[string]string{{
		"name": "LinuxDO", "type": "epay", "min_topup": "5",
	}})

	response := httptest.NewRecorder()
	context, _ := gin.CreateTestContext(response)
	assert.False(t, requirePaymentMethodTopUpWithinLimit(context, "epay", 4))
	assert.Contains(t, response.Body.String(), "5")
}

func TestRequirePaymentMethodTopUpWithinLimitUsesCreditedUSD(t *testing.T) {
	gin.SetMode(gin.TestMode)
	withPaymentMethods(t, []map[string]string{{
		"name": "LinuxDO", "type": "epay", "max_topup": "2.5",
	}})
	previousDisplayType := operation_setting.GetGeneralSetting().QuotaDisplayType
	t.Cleanup(func() {
		operation_setting.GetGeneralSetting().QuotaDisplayType = previousDisplayType
	})

	operation_setting.GetGeneralSetting().QuotaDisplayType = operation_setting.QuotaDisplayTypeUSD
	allowedResponse := httptest.NewRecorder()
	allowedContext, _ := gin.CreateTestContext(allowedResponse)
	assert.True(t, requirePaymentMethodTopUpWithinLimit(allowedContext, "epay", 2))

	blockedResponse := httptest.NewRecorder()
	blockedContext, _ := gin.CreateTestContext(blockedResponse)
	assert.False(t, requirePaymentMethodTopUpWithinLimit(blockedContext, "epay", 3))
	assert.Contains(t, blockedResponse.Body.String(), "2.5")

	operation_setting.GetGeneralSetting().QuotaDisplayType = operation_setting.QuotaDisplayTypeTokens
	tokenResponse := httptest.NewRecorder()
	tokenContext, _ := gin.CreateTestContext(tokenResponse)
	overLimitTokens := int64(3 * common.QuotaPerUnit)
	assert.False(t, requirePaymentMethodTopUpWithinLimit(tokenContext, "epay", overLimitTokens))
}

func TestRequirePaymentMethodCreditedQuotaWithinLimit(t *testing.T) {
	gin.SetMode(gin.TestMode)
	withPaymentMethods(t, []map[string]string{{
		"name": "Creem", "type": "creem", "max_topup": "5",
	}})

	response := httptest.NewRecorder()
	context, _ := gin.CreateTestContext(response)
	assert.False(t, requirePaymentMethodCreditedQuotaWithinLimit(
		context,
		"creem",
		int64(6*common.QuotaPerUnit),
	))
	assert.Contains(t, response.Body.String(), "5")
}
