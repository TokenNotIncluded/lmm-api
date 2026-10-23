package assistant_services

import (
	"bytes"
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/json"
	"errors"
	"strings"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

type Challenge struct {
	ID            string
	URL           string
	PayloadSHA256 string
	ExpiresAt     int64
}
type BrowserProof struct {
	Confirmed     bool
	UserID        int64
	OperationID   string
	PayloadSHA256 string
	ReceiptID     string
	ExpiresAt     int64
}

// Implemented by the authenticated Core private client. This module neither
// signs proofs nor accepts a model-supplied proof/confirm flag as human consent.
type ConfirmationAuthority interface {
	Prepare(context.Context, string, string, string, string, json.RawMessage) (Challenge, error)
	Verify(context.Context, string, string, string, string) (BrowserProof, error)
}
type ConfirmationApply func(context.Context, *sql.Tx, access.Principal, json.RawMessage) (any, error)
type confirmationAction struct {
	minimumLevel int32
	apply        ConfirmationApply
}

func (s *Service) WithConfirmationAction(name string, minimumLevel int32, apply ConfirmationApply) (*Service, error) {
	if s == nil || name == "" || len(name) > 80 || minimumLevel < 0 || minimumLevel > 6 || apply == nil {
		return nil, access.ErrInvalid
	}
	if _, exists := s.confirmations[name]; exists {
		return nil, access.ErrConflict
	}
	out := *s
	out.confirmations = make(map[string]confirmationAction, len(s.confirmations)+1)
	for key, value := range s.confirmations {
		out.confirmations[key] = value
	}
	out.confirmations[name] = confirmationAction{minimumLevel, apply}
	return &out, nil
}
func (s *Service) Prepare(ctx context.Context, in assistant.AgentInvocation, payload json.RawMessage) (assistant.AgentToolResult, error) {
	var object map[string]json.RawMessage
	if decode(payload, &object) != nil || object == nil {
		return assistant.AgentToolResult{}, access.ErrInvalid
	}
	return s.execute(ctx, in, &payload)
}
func (s *Service) prepareInTx(ctx context.Context, tx *sql.Tx, in assistant.AgentInvocation, p access.Principal, session [32]byte, payload json.RawMessage) (assistant.AgentToolResult, error) {
	action, ok := s.confirmations[in.Call.Scope.Name]
	if !ok || p.Level < action.minimumLevel {
		return assistant.AgentToolResult{}, access.ErrForbidden
	}
	if access.Missing(s.dependencies.Confirmations) {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	if len(payload) > 16384 || !json.Valid(payload) {
		return assistant.AgentToolResult{}, access.ErrInvalid
	}
	payloadSHA := digest(payload)
	challenge, err := s.dependencies.Confirmations.Prepare(ctx, in.Credential, in.Call.OperationID, in.Call.Scope.Name, in.Call.ArgumentsSHA256, append(json.RawMessage(nil), payload...))
	if err != nil {
		return assistant.AgentToolResult{}, err
	}
	now := s.now().Unix()
	if challenge.ID == "" || len(challenge.ID) > 256 || !validSHA(challenge.PayloadSHA256) || challenge.ExpiresAt <= now || challenge.ExpiresAt > now+900 || !strings.HasPrefix(challenge.URL, "/") || strings.HasPrefix(challenge.URL, "//") || strings.ContainsAny(challenge.URL, "\x00\r\n") {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	id := digest([]byte("assistant-confirmation:" + in.Call.OperationID))
	_, err = tx.ExecContext(ctx, `INSERT INTO lmm_assistant.service_confirmations(id,operation_id,user_id,session_sha256,tool_name,minimum_level,tool_version,challenge_id,challenge_url,payload_sha256,core_draft_sha256,payload,payload_bytes,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)`, id, in.Call.OperationID, p.UserID, session[:], in.Call.Scope.Name, action.minimumLevel, in.Call.Scope.Version, challenge.ID, challenge.URL, payloadSHA, challenge.PayloadSHA256, string(payload), []byte(payload), challenge.ExpiresAt)
	if err != nil {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	return result(in.Call.OperationID, assistant.AgentToolPending, map[string]any{"ok": true, "status": "awaiting_confirmation", "confirmation_id": id, "confirmation_url": "/api/assistant/services/confirmations/" + id, "expires_at": challenge.ExpiresAt, "applied": false}, id)
}

type ConfirmationView struct {
	ID          string          `json:"id"`
	OperationID string          `json:"operation_id"`
	Tool        string          `json:"tool"`
	BrowserURL  string          `json:"browser_url"`
	Payload     json.RawMessage `json:"payload"`
	ExpiresAt   int64           `json:"expires_at"`
	ConsumedAt  *int64          `json:"consumed_at"`
	Result      json.RawMessage `json:"result,omitempty"`
}

func (s *Service) Confirmation(ctx context.Context, credential, id string) (ConfirmationView, error) {
	p, err := s.principal(ctx, credential)
	if err != nil {
		return ConfirmationView{}, err
	}
	var view ConfirmationView
	var session []byte
	err = s.db.QueryRowContext(ctx, `SELECT id,operation_id,tool_name,challenge_url,payload,expires_at,consumed_at,result,session_sha256 FROM lmm_assistant.service_confirmations WHERE id=$1 AND user_id=$2`, id, p.UserID).Scan(&view.ID, &view.OperationID, &view.Tool, &view.BrowserURL, &view.Payload, &view.ExpiresAt, &view.ConsumedAt, &view.Result, &session)
	if errors.Is(err, sql.ErrNoRows) {
		return view, access.ErrNotFound
	}
	if err != nil {
		return view, access.ErrUnavailable
	}
	current := sha256.Sum256([]byte(credential))
	if !bytes.Equal(session, current[:]) {
		return view, access.ErrForbidden
	}
	return view, nil
}

// ConfirmAfterProof is a browser-only integration boundary, never a model tool.
// The Core verifier must observe an independently redeemed human challenge.
// Consumption, the registered domain write, and its receipt commit together.
func (s *Service) ConfirmAfterProof(ctx context.Context, credential, id string) (json.RawMessage, error) {
	if access.Missing(s.dependencies.Confirmations) {
		return nil, access.ErrUnavailable
	}
	p, err := s.principal(ctx, credential)
	if err != nil {
		return nil, err
	}
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{})
	if err != nil {
		return nil, access.ErrUnavailable
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, `SELECT pg_advisory_xact_lock(hashtextextended($1,0))`, "assistant-services:"+stringID(p.UserID)); err != nil {
		return nil, access.ErrUnavailable
	}
	var operation, tool, version, challenge, payloadSHA, coreDraftSHA string
	var level int32
	var user, expires int64
	var session, payload, savedResult []byte
	var consumed sql.NullInt64
	err = tx.QueryRowContext(ctx, `SELECT operation_id,tool_name,tool_version,challenge_id,payload_sha256,core_draft_sha256,minimum_level,user_id,expires_at,session_sha256,payload_bytes,consumed_at,result FROM lmm_assistant.service_confirmations WHERE id=$1 FOR UPDATE`, id).Scan(&operation, &tool, &version, &challenge, &payloadSHA, &coreDraftSHA, &level, &user, &expires, &session, &payload, &consumed, &savedResult)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, access.ErrNotFound
	}
	if err != nil {
		return nil, access.ErrUnavailable
	}
	current := sha256.Sum256([]byte(credential))
	if user != p.UserID || !bytes.Equal(session, current[:]) || p.Level < level {
		return nil, access.ErrForbidden
	}
	if consumed.Valid {
		return append(json.RawMessage(nil), savedResult...), nil
	}
	if expires <= s.now().Unix() {
		return nil, access.ErrConflict
	}
	if digest(payload) != payloadSHA {
		return nil, access.ErrUnavailable
	}
	action, ok := s.confirmations[tool]
	if !ok || action.minimumLevel != level || version != assistant.FixedMainToolVersion {
		return nil, access.ErrConflict
	}
	proof, err := s.dependencies.Confirmations.Verify(ctx, credential, challenge, operation, coreDraftSHA)
	if err != nil {
		return nil, err
	}
	if !proof.Confirmed || proof.UserID != p.UserID || proof.OperationID != operation || proof.PayloadSHA256 != coreDraftSHA || proof.ReceiptID == "" || proof.ExpiresAt <= s.now().Unix() {
		return nil, access.ErrForbidden
	}
	// Recheck immediately after remote verification; a revoked browser cannot
	// apply a result merely because an earlier boundary authorized it.
	p, err = s.principal(ctx, credential)
	if err != nil {
		return nil, err
	}
	if p.UserID != user || p.Level < level {
		return nil, access.ErrForbidden
	}
	value, err := action.apply(ctx, tx, p, payload)
	if err != nil {
		return nil, err
	}
	raw, err := json.Marshal(map[string]any{"ok": true, "applied": true, "operation_id": operation, "confirmation_id": id, "proof_receipt_id": proof.ReceiptID, "result": value})
	if err != nil || len(raw) > 64<<10 {
		return nil, access.ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, `UPDATE lmm_assistant.service_confirmations SET consumed_at=$2,result=$3 WHERE id=$1 AND consumed_at IS NULL`, id, s.now().Unix(), string(raw)); err != nil {
		return nil, access.ErrUnavailable
	}
	if tx.Commit() != nil {
		return nil, access.ErrUnavailable
	}
	return raw, nil
}

func validSHA(value string) bool {
	if len(value) != 64 {
		return false
	}
	for _, r := range value {
		if !(r >= '0' && r <= '9' || r >= 'a' && r <= 'f') {
			return false
		}
	}
	return true
}
