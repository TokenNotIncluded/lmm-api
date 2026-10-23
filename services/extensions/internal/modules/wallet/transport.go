// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package wallet

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"path/filepath"
	"strings"
	"time"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/access"
)

type CoreClient struct {
	client    *http.Client
	hostToken string
}

func NewCoreClient(socket string, hostToken []byte) (*CoreClient, error) {
	if !filepath.IsAbs(socket) || filepath.Clean(socket) != socket || strings.ContainsAny(socket, "\x00\r\n") || len(hostToken) < 32 || len(hostToken) > 256 {
		return nil, ErrInvalid
	}
	for _, b := range hostToken {
		if b < 33 || b > 126 {
			return nil, ErrInvalid
		}
	}
	d := net.Dialer{Timeout: 2 * time.Second}
	tr := &http.Transport{DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) { return d.DialContext(ctx, "unix", socket) }, MaxConnsPerHost: 4, MaxIdleConnsPerHost: 2, IdleConnTimeout: 30 * time.Second, ResponseHeaderTimeout: 5 * time.Second}
	return &CoreClient{&http.Client{Transport: tr, Timeout: 10 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}, string(hostToken)}, nil
}
func (c *CoreClient) Close() { c.client.CloseIdleConnections() }
func (c *CoreClient) call(ctx context.Context, session, action string, in, out any) error {
	if session == "" || len(session) > 256 || strings.ContainsAny(session, "\r\n") {
		return ErrUnauthorized
	}
	body, e := json.Marshal(in)
	if e != nil {
		return ErrInvalid
	}
	r, e := http.NewRequestWithContext(ctx, "POST", "http://core-wallet/internal/wallet/v1/"+action, bytes.NewReader(body))
	if e != nil {
		return ErrInvalid
	}
	r.Header.Set("Authorization", "Bearer "+c.hostToken)
	r.Header.Set("X-Lmm-User-Credential", session)
	r.Header.Set("Content-Type", "application/json")
	response, e := c.client.Do(r)
	if e != nil {
		return ErrUnknown
	}
	defer response.Body.Close()
	switch response.StatusCode {
	case 200:
	case 400:
		return ErrInvalid
	case 401:
		return ErrUnauthorized
	case 403:
		return ErrForbidden
	case 404:
		return ErrNotFound
	case 409:
		return ErrConflict
	default:
		return ErrUnknown
	}
	raw, e := io.ReadAll(io.LimitReader(response.Body, 131073))
	if e != nil || len(raw) > 131072 {
		return ErrUnknown
	}
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.DisallowUnknownFields()
	if dec.Decode(out) != nil {
		return ErrUnknown
	}
	var trailing any
	if dec.Decode(&trailing) != io.EOF {
		return ErrUnknown
	}
	return nil
}
func (c *CoreClient) Balance(ctx context.Context, s string) (out Balance, e error) {
	e = c.call(ctx, s, "balance", struct{}{}, &out)
	return
}
func (c *CoreClient) Prepare(ctx context.Context, s string, in PrepareInput) (out Transfer, e error) {
	e = c.call(ctx, s, "prepare", in, &out)
	return
}
func (c *CoreClient) List(ctx context.Context, s string, before int64) (out []Transfer, e error) {
	e = c.call(ctx, s, "list", map[string]int64{"before_id": before}, &out)
	return
}
func (c *CoreClient) Cancel(ctx context.Context, s string, id int64) (out Transfer, e error) {
	e = c.call(ctx, s, "cancel", map[string]int64{"id": id}, &out)
	return
}
func (c *CoreClient) Inspect(ctx context.Context, s, token string) (out Receipt, e error) {
	e = c.call(ctx, s, "inspect", map[string]string{"token": token}, &out)
	return
}
func (c *CoreClient) Claim(ctx context.Context, s, token string) (out Receipt, e error) {
	e = c.call(ctx, s, "claim", map[string]string{"token": token}, &out)
	return
}

var _ Authority = (*CoreClient)(nil)

func (c *CoreClient) QueryTransferByRequestKey(ctx context.Context, s, key string) (out Transfer, e error) {
	e = c.call(ctx, s, "query", map[string]string{"request_key": key}, &out)
	return
}

// Ready is host-only and performs no user or financial operation.
func (c *CoreClient) Ready(ctx context.Context) error {
	r, e := http.NewRequestWithContext(ctx, "POST", "http://core-wallet/internal/wallet/v1/ready", strings.NewReader("{}"))
	if e != nil {
		return ErrInvalid
	}
	r.Header.Set("Authorization", "Bearer "+c.hostToken)
	r.Header.Set("X-Lmm-Protocol", "1")
	r.Header.Set("Content-Type", "application/json")
	response, e := c.client.Do(r)
	if e != nil {
		return ErrUnknown
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return ErrUnknown
	}
	raw, e := io.ReadAll(io.LimitReader(response.Body, 4097))
	if e != nil || len(raw) > 4096 {
		return ErrUnknown
	}
	var ack struct {
		Protocol int    `json:"protocol"`
		Service  string `json:"service"`
		Ready    bool   `json:"ready"`
	}
	if access.Decode(raw, &ack) != nil {
		return ErrUnknown
	}
	var fields map[string]json.RawMessage
	if json.Unmarshal(raw, &fields) != nil || len(fields) != 3 || fields["protocol"] == nil || fields["service"] == nil || fields["ready"] == nil || ack.Protocol != 1 || ack.Service != "wallet" || !ack.Ready {
		return ErrUnknown
	}
	return nil
}
