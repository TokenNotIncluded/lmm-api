// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"
)

func TestLegacyCredentialBindingCopyAndValidationInvalidation(t *testing.T) {
	m, _, mock := webFixture(t)
	service, err := m.webSave(Principal{User: "1", Account: "1"}, 1, "", webInput(mock.server.URL+"/mcp", "private"))
	if err != nil {
		t.Fatal(err)
	}
	p := Principal{User: "1", Account: "1"}
	version := service.DraftVersionID
	if err = m.store.update(func(d *database) error {
		v := d.Web.Versions[version]
		v.ValidationDigest = v.Digest
		d.Web.Versions[version] = v
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	metadata, err := m.webConfigureCredential(p, 1, service.ID, WebCredentialInput{VersionID: version, Mode: "bearer", Secret: "synthetic-provider-key"})
	if err != nil || !metadata.Configured || metadata.Mode != "bearer" {
		t.Fatal(metadata, err)
	}
	public, _ := json.Marshal(metadata)
	if strings.Contains(string(public), "synthetic-provider-key") {
		t.Fatal("secret exposed in metadata")
	}
	if err = m.store.view(func(d database) error {
		if d.Web.Versions[version].ValidationDigest != "" {
			return ErrConflict
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	if _, err = m.webCredentialMetadata(2, service.ID, version); !errors.Is(err, ErrMissing) {
		t.Fatal("cross-user credential exposed", err)
	}
	updated, err := m.webSave(p, 1, service.ID, webInput(mock.server.URL+"/mcp", "private"))
	if err != nil {
		t.Fatal(err)
	}
	if _, err = m.webConfigureCredential(p, 1, service.ID, WebCredentialInput{VersionID: updated.DraftVersionID, Mode: "bearer", CopyFromVersionID: version}); err != nil {
		t.Fatal(err)
	}
	if _, err = m.webConfigureCredential(p, 1, service.ID, WebCredentialInput{VersionID: version, Mode: "none"}); !errors.Is(err, ErrConflict) {
		t.Fatal("changed old immutable version credential", err)
	}
	if _, err = m.webConfigureCredential(p, 1, service.ID, WebCredentialInput{VersionID: updated.DraftVersionID, Mode: "none"}); err != nil {
		t.Fatal(err)
	}
	metadata, err = m.webCredentialMetadata(1, service.ID, updated.DraftVersionID)
	if err != nil || metadata.Configured || metadata.Mode != "none" {
		t.Fatal(metadata, err)
	}
}
func TestLegacyCredentialsRejectInternalTokensAndControls(t *testing.T) {
	for _, secret := range []string{"lmm_at_secret", "lmm_rt_secret", "lmm_market_secret", "lmm_mcp_secret", "lmms_secret", "lmmk_secret", "lmmt_secret", "abc\nsecret", "abc\x00secret"} {
		if validWebCredential("bearer", secret) || validWebCredential("api_key", secret) {
			t.Fatal("internal or control credential accepted")
		}
	}
	if validWebCredential("bearer", "abc def") || !validWebCredential("api_key", "abc def") || validWebCredential("none", "secret") {
		t.Fatal("mode validation changed")
	}
}
func TestLegacyCredentialRemoteInspectionUsesBoundSecretOnly(t *testing.T) {
	m, _, mock := webFixture(t)
	for _, mode := range []string{"bearer", "api_key"} {
		mock.mu.Lock()
		mock.seen = nil
		mock.mu.Unlock()
		tools, err := m.webRemoteCredential(context.Background(), mock.server.URL+"/mcp", webCredential{Mode: mode, Secret: "synthetic-external-secret"})
		if err != nil || len(tools) != 1 {
			t.Fatal("credentialed discovery failed", err)
		}
		mock.mu.Lock()
		seen := append([]string(nil), mock.seen...)
		mock.mu.Unlock()
		want := "synthetic-external-secret"
		if mode == "bearer" {
			want = "Bearer " + want
		}
		if len(seen) < 2 {
			t.Fatal("no real initialize/list requests")
		}
		for _, header := range seen {
			if header != want {
				t.Fatal("wrong outbound auth mode")
			}
		}
		public, _ := json.Marshal(tools)
		if strings.Contains(string(public), "synthetic-external-secret") {
			t.Fatal("secret exposed in discovered tools")
		}
	}
	if mock.calls.Load() != 0 {
		t.Fatal("inspection invoked tool side effect")
	}
}
