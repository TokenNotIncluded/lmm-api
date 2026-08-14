package model

import (
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestRetiredAssistantWeeklyCreditOptionIsRuntimeZero(t *testing.T) {
	common.OptionMapRWMutex.Lock()
	previous, existed := common.OptionMap[setting.AssistantWeeklyCreditUSDOptionKey]
	common.OptionMapRWMutex.Unlock()
	t.Cleanup(func() {
		common.OptionMapRWMutex.Lock()
		defer common.OptionMapRWMutex.Unlock()
		if existed {
			common.OptionMap[setting.AssistantWeeklyCreditUSDOptionKey] = previous
		} else {
			delete(common.OptionMap, setting.AssistantWeeklyCreditUSDOptionKey)
		}
	})

	require.NoError(t, updateOptionMap(setting.AssistantWeeklyCreditUSDOptionKey, "1"))
	common.OptionMapRWMutex.RLock()
	value := common.OptionMap[setting.AssistantWeeklyCreditUSDOptionKey]
	common.OptionMapRWMutex.RUnlock()
	assert.Equal(t, "0", value)
}
