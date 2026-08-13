// Read-only admin status endpoints for the dynamic pricing feature.
package controller

import (
	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/pkg/dynamic_pricing"
	"github.com/QuantumNous/new-api/setting/dynamic_pricing_setting"

	"github.com/gin-gonic/gin"
)

// GetDynamicPricingStatus returns a read-only snapshot of the dynamic pricing
// feature for the admin console: whether it is enabled, the full setting
// (none of its fields are sensitive), and the per-model runtime state
// (factor, load/cost EMA, last tick time).
func GetDynamicPricingStatus(c *gin.Context) {
	s := dynamic_pricing_setting.GetSetting()

	models := make(map[string]gin.H, 0)
	for _, modelName := range dynamic_pricing.AllModels() {
		state, ok := dynamic_pricing.GetState(modelName)
		if !ok || state == nil {
			continue
		}
		models[modelName] = gin.H{
			"factor":     state.Factor,
			"load_ema":   state.LoadEMA,
			"cost_ema":   state.CostEMA,
			"updated_at": state.UpdatedAt,
		}
	}

	common.ApiSuccess(c, gin.H{
		"enabled": s.Enabled,
		"setting": s,
		"models":  models,
	})
}
