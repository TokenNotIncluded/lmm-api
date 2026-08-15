/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

package operation_setting

import (
	"strings"

	"github.com/LIghtJUNction/api.lmm.best/setting/config"
)

// PublicRelaySetting controls the group assigned to user-submitted public
// relay channels. The value is administrator-owned; submissions never choose
// a billing/routing group themselves.
type PublicRelaySetting struct {
	Group string `json:"group"`
}

var publicRelaySetting = PublicRelaySetting{Group: "FREE"}

func init() {
	config.GlobalConfig.Register("public_relay_setting", &publicRelaySetting)
}

func GetPublicRelaySetting() *PublicRelaySetting {
	return &publicRelaySetting
}

func GetPublicRelayGroup() string {
	group := strings.TrimSpace(publicRelaySetting.Group)
	if group == "" {
		return "FREE"
	}
	return group
}
