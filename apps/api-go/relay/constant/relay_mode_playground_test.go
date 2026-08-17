/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

package constant

import (
	"testing"

	"github.com/stretchr/testify/require"
)

func TestPath2RelayModeRecognizesPlaygroundImageEdits(t *testing.T) {
	require.Equal(t, RelayModeImagesEdits, Path2RelayMode("/pg/images/edits"))
}
