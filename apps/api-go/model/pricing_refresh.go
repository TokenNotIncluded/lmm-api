package model

import (
	"fmt"

	"github.com/LIghtJUNction/api.lmm.best/common"
)

// RefreshPricing 强制立即重新计算与定价相关的缓存。
// 该方法用于需要最新数据的内部管理 API，
// 因此会绕过默认的 1 分钟延迟刷新。
func RefreshPricing() error {
	if err := refreshPricingNow(); err != nil {
		err = fmt.Errorf("force refresh pricing cache: %w", err)
		common.SysLog(err.Error())
		return err
	}
	return nil
}
