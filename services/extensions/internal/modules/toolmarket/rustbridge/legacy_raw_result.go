// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	market "github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/toolmarket"
	"io"
	"net/http"
)

// Only the closed terminal-proof leaves accepts a 2 MiB original MCP result encoded
// as base64. Native v1 and ordinary v2 request limits remain unchanged.
func (a *Adapter) requestLegacySettlement(ctx context.Context, in, out any) error {
	return a.requestLegacyResult(ctx, "calls/settle", in, out)
}
func (a *Adapter) requestLegacyResult(ctx context.Context, action string, in, out any) error {
	if action != "calls/settle" && action != "calls/release" && action != "fixed-witness/settle" && action != "fixed-witness/release" {
		return market.ErrInvalid
	}
	if a == nil || a.client == nil {
		return market.ErrUnavailable
	}
	raw, e := json.Marshal(in)
	if e != nil || len(raw) > 2850000 {
		return market.ErrInvalid
	}
	req, e := http.NewRequestWithContext(ctx, http.MethodPost, "http://core-toolmarket/internal/tool-market/v2/"+action, bytes.NewReader(raw))
	if e != nil {
		return market.ErrUnavailable
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+a.token)
	req.Header.Set("X-Lmm-Protocol", "2")
	response, e := a.client.Do(req)
	if e != nil {
		return market.ErrUnavailable
	}
	defer response.Body.Close()
	body, e := io.ReadAll(io.LimitReader(response.Body, 65537))
	if e != nil || len(body) > 65536 {
		return market.ErrUnavailable
	}
	if response.StatusCode != http.StatusOK {
		switch response.StatusCode {
		case 400:
			return market.ErrInvalid
		case 401:
			return market.ErrAuth
		case 403:
			return market.ErrDenied
		case 404:
			return market.ErrMissing
		case 409:
			return market.ErrConflict
		default:
			return market.ErrUnavailable
		}
	}
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.DisallowUnknownFields()
	if decoder.Decode(out) != nil {
		return market.ErrUnavailable
	}
	var extra any
	if !errors.Is(decoder.Decode(&extra), io.EOF) {
		return market.ErrUnavailable
	}
	return nil
}
