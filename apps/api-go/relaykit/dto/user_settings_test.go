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
package dto

import (
	"strings"
	"testing"
)

func TestValidateSidebarModules(t *testing.T) {
	tests := []struct {
		name  string
		value string
		valid bool
	}{
		{name: "empty", value: "", valid: true},
		{name: "legacy object", value: `{"chat":{"enabled":true}}`, valid: true},
		{name: "preferences envelope", value: `{"modules":{"chat":{"enabled":true}},"preferences":{"density":"compact","default_route":"/dashboard/overview","hidden":[]}}`, valid: true},
		{name: "invalid json", value: `{`, valid: false},
		{name: "invalid density", value: `{"preferences":{"density":"dense"}}`, valid: false},
		{name: "external route", value: `{"preferences":{"default_route":"//evil.example"}}`, valid: false},
		{name: "oversized", value: `{"note":"` + strings.Repeat("x", SidebarModulesMaxBytes) + `"}`, valid: false},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			err := ValidateSidebarModules(test.value)
			if (err == nil) != test.valid {
				t.Fatalf("ValidateSidebarModules() error = %v, valid = %v", err, test.valid)
			}
		})
	}
}
