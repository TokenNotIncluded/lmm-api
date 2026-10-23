// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"

	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

// AuthorizeBuiltin consumes the Core-owned binding on the private market UDS.
// An ambiguous response never authorizes a downstream call and is never retried.
func (a *Adapter) AuthorizeBuiltin(ctx context.Context, session string, in market.BuiltinAuthorizationRequest) (market.BuiltinAuthorization, error) {
	var out market.BuiltinAuthorization
	if a == nil || a.client == nil {
		return out, market.ErrUnavailable
	}
	raw, e := json.Marshal(in)
	if e != nil || len(raw) > 80<<10 {
		return out, market.ErrInvalid
	}
	req, e := http.NewRequestWithContext(ctx, http.MethodPost, "http://core-toolmarket/internal/builtins/v1/authorize", bytes.NewReader(raw))
	if e != nil {
		return out, market.ErrInvalid
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+a.token)
	req.Header.Set("X-Lmm-Protocol", "1")
	req.Header.Set("X-Lmm-User-Credential", session)
	res, e := a.client.Do(req)
	if e != nil {
		return out, market.ErrUnavailable
	}
	defer res.Body.Close()
	body, e := io.ReadAll(io.LimitReader(res.Body, 16385))
	if e != nil || len(body) > 16384 {
		return out, market.ErrUnavailable
	}
	if res.StatusCode != http.StatusOK {
		switch res.StatusCode {
		case 400:
			return out, market.ErrInvalid
		case 401, 403:
			return out, market.ErrDenied
		case 409:
			return out, market.ErrConflict
		default:
			return out, market.ErrUnavailable
		}
	}
	d := json.NewDecoder(bytes.NewReader(body))
	d.DisallowUnknownFields()
	if d.Decode(&out) != nil {
		return out, market.ErrUnavailable
	}
	var extra any
	if d.Decode(&extra) != io.EOF {
		return market.BuiltinAuthorization{}, market.ErrUnavailable
	}
	return out, nil
}
