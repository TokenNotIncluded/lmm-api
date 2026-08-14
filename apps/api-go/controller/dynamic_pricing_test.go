package controller

import (
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/stretchr/testify/require"
)

func TestDynamicPricingRequestFactorRangeIncludesImmediateChannelFloors(t *testing.T) {
	channels := []gin.H{
		{"cost": 2.0},
		{"cost": 3.0},
		{"cost": 0.0},
	}

	minimum, maximum := dynamicPricingRequestFactorRange(1.5, 1, 1.2, channels)
	require.InDelta(t, 2.4, minimum, 1e-9)
	require.InDelta(t, 3.6, maximum, 1e-9)
}

func TestDynamicPricingRequestFactorRangeFallsBackToEngineWithoutCosts(t *testing.T) {
	minimum, maximum := dynamicPricingRequestFactorRange(1.75, 1, 1.2, nil)
	require.Equal(t, 1.75, minimum)
	require.Equal(t, 1.75, maximum)
}
