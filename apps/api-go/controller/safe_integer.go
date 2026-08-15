package controller

import "strconv"

// parsePlatformUint parses an identifier/config value at the width of the
// platform value it will be stored in. It prevents ParseUint's 64-bit result
// from silently wrapping when it is converted to uint on 32-bit builds.
func parsePlatformUint(value string) (uint, error) {
	parsed, err := strconv.ParseUint(value, 10, strconv.IntSize)
	if err != nil {
		return 0, err
	}
	return uint(parsed), nil
}
