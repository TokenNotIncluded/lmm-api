// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
// Adapted from fixed main 3948a0a model/tool_market_credentials.go.
package toolmarket

import (
	"context"
	"encoding/json"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/redaction"
	"strings"
	"unicode"
	"unicode/utf8"
)

// Credential material is private to the encrypted module snapshot. It never
// enters a public version, digest, audit event, metadata response or tool output.
type webCredential struct {
	OwnerID        int64  `json:"owner_id"`
	ServiceID      string `json:"service_id"`
	VersionID      string `json:"version_id"`
	EndpointDigest string `json:"endpoint_digest"`
	Mode           string `json:"mode"`
	Secret         string `json:"secret"`
	UpdatedAt      int64  `json:"updated_at"`
}
type WebCredentialMetadata struct {
	Mode       string `json:"mode"`
	Configured bool   `json:"configured"`
	UpdatedAt  int64  `json:"updated_at"`
}
type WebCredentialInput struct {
	VersionID         string `json:"version_id"`
	Mode              string `json:"mode"`
	Secret            string `json:"secret"`
	CopyFromVersionID string `json:"copy_from_version_id"`
}

func validWebCredential(mode, secret string) bool {
	if mode == "none" {
		return secret == ""
	}
	if mode != "bearer" && mode != "api_key" || len(secret) == 0 || len(secret) > 4096 || !utf8.ValidString(secret) || strings.TrimSpace(secret) == "" {
		return false
	}
	for _, c := range secret {
		if unicode.IsControl(c) || mode == "bearer" && unicode.IsSpace(c) {
			return false
		}
	}
	for _, prefix := range []string{"lmm_at_", "lmm_rt_", "lmm_market_", "lmm_mcp_", "lmm_drawing_mcp_", "lmm_bounty_mcp_", "lmms_", "lmmk_", "lmmt_"} {
		if strings.HasPrefix(strings.TrimSpace(secret), prefix) {
			return false
		}
	}
	return true
}
func webResolveCredential(d database, user int64, service, version, endpoint string) (webCredential, error) {
	if d.Web == nil {
		return webCredential{}, ErrMissing
	}
	w := d.Web
	s, ok := w.Services[service]
	v, exists := w.Versions[version]
	if !ok || !exists || s.OwnerID != user || v.Version.ServiceID != service {
		return webCredential{}, ErrMissing
	}
	if v.Version.ExecutionType != "remote" || endpoint != "" && v.Version.Endpoint != endpoint {
		return webCredential{}, ErrConflict
	}
	record, ok := w.Credentials[version]
	if !ok {
		return webCredential{Mode: "none"}, nil
	}
	if record.OwnerID != user || record.ServiceID != service || record.VersionID != version || record.EndpointDigest != digest(v.Version.Endpoint) || !validWebCredential(record.Mode, record.Secret) || record.Mode == "none" {
		return webCredential{}, ErrConflict
	}
	return record, nil
}
func (m *Module) webCredentialMetadata(user int64, service, version string) (WebCredentialMetadata, error) {
	if !validID.MatchString(version) {
		return WebCredentialMetadata{}, ErrInvalid
	}
	var out WebCredentialMetadata
	err := m.store.view(func(d database) error {
		r, err := webResolveCredential(d, user, service, version, "")
		if err != nil {
			return err
		}
		out = WebCredentialMetadata{Mode: r.Mode, Configured: r.Mode != "none", UpdatedAt: r.UpdatedAt}
		return nil
	})
	return out, err
}
func (m *Module) webConfigureCredential(p Principal, user int64, service string, in WebCredentialInput) (WebCredentialMetadata, error) {
	if !validID.MatchString(in.VersionID) || in.CopyFromVersionID != "" && (!validID.MatchString(in.CopyFromVersionID) || in.CopyFromVersionID == in.VersionID || in.Secret != "" || in.Mode == "none") || in.CopyFromVersionID == "" && !validWebCredential(in.Mode, in.Secret) {
		return WebCredentialMetadata{}, ErrInvalid
	}
	err := m.store.update(func(d *database) error {
		w := webState(d)
		s, err := webOwned(w, user, service)
		if err != nil {
			return err
		}
		v, ok := w.Versions[in.VersionID]
		if !ok || s.DraftVersionID != in.VersionID || v.Version.ServiceID != service || v.Version.Status != "draft" || v.Version.ExecutionType != "remote" {
			return ErrConflict
		}
		if _, err := m.net.url(v.Version.Endpoint); err != nil {
			return err
		}
		mode, secret := in.Mode, in.Secret
		if in.CopyFromVersionID != "" {
			prior, err := webResolveCredential(*d, user, service, in.CopyFromVersionID, v.Version.Endpoint)
			if err != nil {
				return err
			}
			if prior.Mode == "none" || mode != "" && mode != prior.Mode {
				return ErrInvalid
			}
			mode, secret = prior.Mode, prior.Secret
		}
		if w.Credentials == nil {
			w.Credentials = map[string]webCredential{}
		}
		if mode == "none" {
			delete(w.Credentials, in.VersionID)
		} else {
			w.Credentials[in.VersionID] = webCredential{OwnerID: user, ServiceID: service, VersionID: in.VersionID, EndpointDigest: digest(v.Version.Endpoint), Mode: mode, Secret: secret, UpdatedAt: m.now().Unix()}
		}
		v.ValidationDigest = ""
		v.RemoteDigests = nil
		w.Versions[in.VersionID] = v
		d.audit(p, "web.credential.configure", service, "ok")
		return nil
	})
	if err != nil {
		return WebCredentialMetadata{}, err
	}
	return m.webCredentialMetadata(user, service, in.VersionID)
}
func (m *Module) webRemoteCredential(ctx context.Context, endpoint string, credential webCredential) ([]Tool, error) {
	ctx, cancel := context.WithTimeout(ctx, m.timeout)
	defer cancel()
	server := Server{Endpoint: endpoint, Auth: "none"}
	secret := Secret{}
	if credential.Mode != "" && credential.Mode != "none" {
		if !validWebCredential(credential.Mode, credential.Secret) {
			return nil, ErrInvalid
		}
		server.Auth = "api_key"
		server.Header = "Authorization"
		if credential.Mode == "api_key" {
			server.Header = "X-Api-Key"
		}
		secret.Access = credential.Secret
	}
	connection, err := m.connectLegacy(ctx, server, secret)
	if err != nil {
		return nil, err
	}
	defer connection.close(ctx)
	tools, err := connection.legacyTools(ctx)
	if err != nil {
		return nil, err
	}
	raw, err := json.Marshal(tools)
	if err != nil {
		return nil, ErrUpstream
	}
	raw, err = redaction.JSON(raw, credential.Secret)
	if err != nil {
		return nil, ErrUpstream
	}
	if json.Unmarshal(raw, &tools) != nil {
		return nil, ErrUpstream
	}
	return tools, nil
}
