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
package console_setting

import (
	"strings"
	"testing"
)

func TestValidateAnnouncementsCountsUnicodeCharacters(t *testing.T) {
	valid := `[{"content":"` + strings.Repeat("公告", 250) + `","publishDate":"2026-08-14T00:00:00Z","type":"default"}]`
	if err := ValidateConsoleSettings(valid, "Announcements"); err != nil {
		t.Fatalf("500 Unicode characters should be accepted: %v", err)
	}

	tooLong := `[{"content":"` + strings.Repeat("公告", 251) + `","publishDate":"2026-08-14T00:00:00Z","type":"default"}]`
	if err := ValidateConsoleSettings(tooLong, "Announcements"); err == nil {
		t.Fatal("501 Unicode characters should be rejected")
	}
}
