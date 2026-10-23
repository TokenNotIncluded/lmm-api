package support

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"path/filepath"
	"strings"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

type CoreCustomerProjection struct {
	client *http.Client
	token  string
}

func NewCoreCustomerProjection(socket string, token []byte) (*CoreCustomerProjection, error) {
	if !filepath.IsAbs(socket) || filepath.Clean(socket) != socket || strings.ContainsAny(socket, "\x00\r\n") || len(token) < 32 || len(token) > 256 {
		return nil, store.ErrInvalid
	}
	for _, b := range token {
		if b < 33 || b > 126 {
			return nil, store.ErrInvalid
		}
	}
	dialer := &net.Dialer{Timeout: 2 * time.Second}
	transport := &http.Transport{DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		return dialer.DialContext(ctx, "unix", socket)
	}, MaxConnsPerHost: 4, MaxIdleConnsPerHost: 2, IdleConnTimeout: 30 * time.Second, ResponseHeaderTimeout: 2 * time.Second}
	return &CoreCustomerProjection{client: &http.Client{Transport: transport, Timeout: 3 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}, token: string(token)}, nil
}
func (p *CoreCustomerProjection) Close() {
	if p != nil && p.client != nil {
		p.client.CloseIdleConnections()
	}
}

func (p *CoreCustomerProjection) Ready(ctx context.Context) error {
	if p == nil || p.client == nil {
		return store.ErrUnavailable
	}
	request, err := http.NewRequestWithContext(ctx, http.MethodPost, "http://core-support/internal/support/v1/capabilities", strings.NewReader("{}"))
	if err != nil {
		return store.ErrUnavailable
	}
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Authorization", "Bearer "+p.token)
	request.Header.Set("X-Lmm-Protocol", "1")
	response, err := p.client.Do(request)
	if err != nil {
		return store.ErrUnavailable
	}
	defer response.Body.Close()
	raw, err := io.ReadAll(io.LimitReader(response.Body, 4097))
	if err != nil || len(raw) > 4096 || response.StatusCode != 200 {
		return store.ErrUnavailable
	}
	var value struct {
		Protocol int      `json:"protocol_major"`
		Features []string `json:"features"`
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	if decoder.Decode(&value) != nil || value.Protocol != 1 || len(value.Features) != 1 || value.Features[0] != "support.customers.integer_credits.v1" {
		return store.ErrUnavailable
	}
	var extra any
	if !errors.Is(decoder.Decode(&extra), io.EOF) {
		return store.ErrUnavailable
	}
	return nil
}
func (p *CoreCustomerProjection) Customers(ctx context.Context, token string, seller int64, relations []CustomerRelationship, q string, offset, limit int) ([]WebCustomer, error) {
	return p.customers(ctx, token, seller, relations, nil, q, offset, limit)
}
func (p *CoreCustomerProjection) Customer(ctx context.Context, token string, seller int64, relations []CustomerRelationship, buyer int64) (WebCustomer, error) {
	if buyer <= 0 || buyer == seller {
		return WebCustomer{}, store.ErrInvalid
	}
	rows, err := p.customers(ctx, token, seller, relations, &buyer, "", 0, 1)
	if err != nil {
		return WebCustomer{}, err
	}
	if len(rows) != 1 || rows[0].BuyerID != buyer {
		return WebCustomer{}, store.ErrForbidden
	}
	return rows[0], nil
}
func (p *CoreCustomerProjection) customers(ctx context.Context, token string, seller int64, relations []CustomerRelationship, buyer *int64, q string, offset, limit int) ([]WebCustomer, error) {
	if p == nil || p.client == nil {
		return nil, store.ErrUnavailable
	}
	if seller <= 0 || len(relations) > 5000 || offset < 0 || offset > 100000 || !store.Page(limit) {
		return nil, store.ErrInvalid
	}
	if relations == nil {
		relations = []CustomerRelationship{}
	}
	body, err := json.Marshal(struct {
		Seller    int64                  `json:"seller_id"`
		Buyer     *int64                 `json:"buyer_id,omitempty"`
		Relations []CustomerRelationship `json:"relationships"`
		Query     string                 `json:"query"`
		Offset    int                    `json:"offset"`
		Limit     int                    `json:"limit"`
	}{seller, buyer, relations, q, offset, limit})
	if err != nil || len(body) > 1048576 {
		return nil, store.ErrInvalid
	}
	request, err := http.NewRequestWithContext(ctx, http.MethodPost, "http://core-support/internal/support/v1/customers", bytes.NewReader(body))
	if err != nil {
		return nil, store.ErrInvalid
	}
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Authorization", "Bearer "+p.token)
	request.Header.Set("X-Lmm-Protocol", "1")
	request.Header.Set("X-Lmm-User-Credential", token)
	response, err := p.client.Do(request)
	if err != nil {
		return nil, store.ErrUnavailable
	}
	defer response.Body.Close()
	raw, err := io.ReadAll(io.LimitReader(response.Body, 1048577))
	if err != nil || len(raw) > 1048576 {
		return nil, store.ErrUnavailable
	}
	if response.StatusCode != http.StatusOK {
		switch response.StatusCode {
		case 400:
			return nil, store.ErrInvalid
		case 401:
			return nil, store.ErrUnauthorized
		case 403:
			return nil, store.ErrForbidden
		default:
			return nil, store.ErrUnavailable
		}
	}
	var rows []struct {
		BuyerID        int64  `json:"buyer_id"`
		DisplayName    string `json:"display_name"`
		OrderCount     int64  `json:"order_count"`
		PaidQuota      int64  `json:"paid_quota"`
		LastOrderAt    int64  `json:"last_order_at"`
		LastOrderID    string `json:"last_order_id"`
		ConversationID string `json:"conversation_id"`
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	if err = decoder.Decode(&rows); err != nil {
		return nil, store.ErrUnavailable
	}
	var extra any
	if !errors.Is(decoder.Decode(&extra), io.EOF) || rows == nil || len(rows) > limit {
		return nil, store.ErrUnavailable
	}
	out := make([]WebCustomer, 0, len(rows))
	for _, row := range rows {
		if row.BuyerID <= 0 || row.BuyerID == seller || row.OrderCount < 0 || row.PaidQuota < 0 {
			return nil, store.ErrUnavailable
		}
		out = append(out, WebCustomer{BuyerID: row.BuyerID, DisplayName: row.DisplayName, OrderCount: row.OrderCount, PaidQuota: row.PaidQuota, LastOrderAt: row.LastOrderAt, LastOrderID: row.LastOrderID, ConversationID: row.ConversationID, Tags: []string{}})
	}
	return out, nil
}
