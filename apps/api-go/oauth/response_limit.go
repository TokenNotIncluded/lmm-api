/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.
*/

package oauth

import (
	"encoding/json"
	"io"

	"github.com/LIghtJUNction/api.lmm.best/common"
)

// oauthResponseBodyMaxBytes keeps provider-controlled OAuth responses small.
// Token and profile payloads are tiny; allowing an unbounded response here
// would let a misbehaving provider turn a login attempt into a memory spike.
const oauthResponseBodyMaxBytes int64 = 1 << 20

// decodeOAuthJSON decodes a provider response only after applying a strict
// byte ceiling. Built-in providers otherwise pass the remote body straight to
// json.Decoder, which can retain attacker-controlled string or array data.
func decodeOAuthJSON(body io.Reader, value any) error {
	data, err := common.ReadAllLimit(body, oauthResponseBodyMaxBytes)
	if err != nil {
		return err
	}
	return json.Unmarshal(data, value)
}
