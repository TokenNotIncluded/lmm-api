package service

import (
	"strings"

	"github.com/LIghtJUNction/api.lmm.best/setting/system_setting"
)

func PaymentReturnURL(suffix string) string {
	base := strings.TrimRight(system_setting.ServerAddress, "/")
	return base + suffix
}
