/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

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
	"errors"
	"net/http"
	"sort"
	"strings"

	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/setting"
	"github.com/LIghtJUNction/api.lmm.best/setting/ratio_setting"
	"github.com/gin-gonic/gin"
)

// GetAssistantModels returns the exact enabled model IDs for an administrator
// selected routing group. The settings page uses this as an enum source so a
// model cannot be configured for a group where it is not actually routable.
func GetAssistantModels(c *gin.Context) {
	group := strings.TrimSpace(c.Query("group"))
	if group == "" {
		group = strings.TrimSpace(setting.GetAssistantSettings().Group)
	}
	if group == "" {
		group = setting.DefaultAssistantGroup
	}
	if !ratio_setting.ContainsGroupRatio(group) {
		writeAssistantError(c, http.StatusBadRequest, "ASSISTANT_INVALID_GROUP", errors.New("assistant routing group must be an existing group"))
		return
	}

	models, err := model.GetGroupEnabledModelsWithError(group)
	if err != nil {
		writeAssistantError(c, http.StatusServiceUnavailable, "ASSISTANT_MODEL_CATALOG_UNAVAILABLE", errors.New("assistant model catalog is temporarily unavailable"))
		return
	}
	sort.Strings(models)
	c.JSON(http.StatusOK, gin.H{
		"success": true,
		"message": "",
		"data":    models,
		"group":   group,
	})
}
