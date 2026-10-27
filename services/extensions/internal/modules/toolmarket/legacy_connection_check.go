// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"mime"
	"net/http"
	"strings"
)

// WebConnectionCheck is an observation, not execution authority or proof that
// MCP discovery succeeded. Upstream 401/403 stay inside this value: HTTP 401/403
// on the LMM API continue to mean LMM session/action authorization failures.
type WebConnectionCheck struct {
	Provider       string `json:"provider"`
	Surface        string `json:"surface"`
	Authentication string `json:"authentication"`
	Status         string `json:"status"`
	UpstreamStatus int    `json:"upstream_status,omitempty"`
}

// Treg's managed team keys work as MCP Bearer credentials, but initialize and
// tools/list may return static definitions without authenticating them. The
// documented installer checks GET /auth/me with X-Treg-Token first. This read
// does not invoke tools or consume provider credit. OAuth resource/audience
// validation and MCP discovery are separate; this path accepts a managed key.
// Contract reference: superdesigndev/treg 9418975f6cb964a0bb397f0d0b8cd5f16068e48a,
// docs/context/architecture/mcp-oauth.md and src/treg/routers/auth.py.
func (m *Module) webConnectionCheck(ctx context.Context, user int64, raw json.RawMessage) (WebConnectionCheck, error) {
	var out WebConnectionCheck
	in, err := payload[struct {
		Provider       string `json:"provider"`
		Endpoint       string `json:"endpoint"`
		Authentication *struct {
			Mode   string `json:"mode"`
			Secret string `json:"secret"`
		} `json:"authentication,omitempty"`
		ServiceID string `json:"service_id,omitempty"`
		VersionID string `json:"version_id,omitempty"`
	}](raw, "provider", "endpoint")
	if err != nil {
		return out, err
	}
	if in.Provider != "treg" || (in.ServiceID == "") != (in.VersionID == "") || in.Authentication != nil && in.ServiceID != "" {
		return out, ErrInvalid
	}
	u, err := m.net.url(in.Endpoint)
	if err != nil {
		return out, err
	}
	if u.RawPath != "" {
		return out, ErrInvalid
	}
	out.Provider, out.Authentication, out.Status = "treg", "not_verified", "invalid_response"
	switch strings.TrimSuffix(u.Path, "/") {
	case "/mcp":
		out.Surface = "team"
	case "/mcp/v2":
		out.Surface = "catalog"
	default:
		return WebConnectionCheck{}, ErrInvalid
	}
	credential := webCredential{}
	if in.Authentication != nil {
		credential.Mode, credential.Secret = in.Authentication.Mode, in.Authentication.Secret
	} else if in.ServiceID != "" {
		if err := m.store.view(func(d database) error {
			var err error
			credential, err = webResolveCredential(d, user, in.ServiceID, in.VersionID, in.Endpoint)
			return err
		}); err != nil {
			return WebConnectionCheck{}, err
		}
	}
	if credential.Mode != "bearer" || !validWebCredential(credential.Mode, credential.Secret) {
		return WebConnectionCheck{}, ErrInvalid
	}

	ctx, cancel := context.WithTimeout(ctx, m.timeout)
	defer cancel()
	u.Path = "/auth/me"
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, u.String(), nil)
	if err != nil {
		return WebConnectionCheck{}, ErrInvalid
	}
	request.Header.Set("Accept", "application/json")
	request.Header.Set("X-Treg-Token", credential.Secret)
	// Use the existing checked-DNS, IP-pinned, no-proxy/no-redirect transport.
	// Unlike network.request, retain the error cause to distinguish timeouts.
	response, err := m.net.client.Do(request)
	if err != nil {
		out.Status = connectionFailureStatus(ctx, err)
		return out, nil
	}
	defer response.Body.Close()
	out.UpstreamStatus = response.StatusCode
	switch response.StatusCode {
	case http.StatusUnauthorized:
		out.Status = "unauthorized"
		return out, nil
	case http.StatusForbidden:
		out.Status = "forbidden"
		return out, nil
	case http.StatusOK:
	default:
		out.Status = "unavailable"
		return out, nil
	}
	mediaType, _, err := mime.ParseMediaType(response.Header.Get("Content-Type"))
	if err != nil || mediaType != "application/json" {
		return out, nil
	}
	const identityLimit = 64 << 10
	body, err := io.ReadAll(io.LimitReader(response.Body, identityLimit+1))
	if err != nil {
		out.Status = connectionFailureStatus(ctx, err)
		return out, nil
	}
	var identity struct {
		Email string `json:"email"`
		OrgID int64  `json:"org_id"`
		Org   string `json:"org"`
	}
	if len(body) > identityLimit || uniqueJSON(body) != nil || json.Unmarshal(body, &identity) != nil || strings.TrimSpace(identity.Email) == "" {
		return out, nil
	}
	if identity.OrgID <= 0 || strings.TrimSpace(identity.Org) == "" {
		out.Status = "team_required"
		return out, nil
	}
	out.Authentication, out.Status = "verified", "verified"
	return out, nil
}

func connectionFailureStatus(ctx context.Context, err error) string {
	if errors.Is(ctx.Err(), context.DeadlineExceeded) || remoteTimeout(err) {
		return "timeout"
	}
	return "network_error"
}
