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
package ratio_setting

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/stretchr/testify/require"
)

func TestGroupGroupRatioRejectsMalformedJSONWithoutClearingLiveMap(t *testing.T) {
	previous := groupGroupRatioMap.ReadAll()
	previousJSON, err := json.Marshal(previous)
	require.NoError(t, err)
	t.Cleanup(func() {
		require.NoError(t, UpdateGroupGroupRatioByJSONString(string(previousJSON)))
	})

	require.NoError(t, UpdateGroupGroupRatioByJSONString(`{"vip":{"default":0.9}}`))
	before := groupGroupRatioMap.ReadAll()

	require.Error(t, CheckGroupGroupRatio(`{"vip":`))
	require.Error(t, UpdateGroupGroupRatioByJSONString(`{"vip":`))
	require.Equal(t, before, groupGroupRatioMap.ReadAll())
	require.Error(t, CheckGroupGroupRatio(`{"vip":{"default":-0.1}}`))
}

func TestGroupWarningValidationRejectsLongKeysAndNull(t *testing.T) {
	longKey := strings.Repeat("x", 65)
	require.Error(t, CheckGroupWarnings(`null`))
	require.Error(t, CheckGroupWarnings(`{"`+longKey+`":{"enabled":false}}`))
}
