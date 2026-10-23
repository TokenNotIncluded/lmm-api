// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
// Extension-owned personal skills, issue records and platform support. These
// records confer no Core identity, permission, model budget or spending rights.
package assistant_services

import (
	"bytes"
	"context"
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	_ "embed"
	"encoding/hex"
	"encoding/json"
	"errors"
	"strings"
	"time"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/access"
)

//go:embed schema.sql
var Schema string

type Service struct {
	db            *sql.DB
	auth          access.Authority
	dependencies  Dependencies
	now           func() time.Time
	confirmations map[string]confirmationAction
	actions       map[string]toolAction
}
type Eligibility struct {
	Eligible  bool   `json:"eligible"`
	ReceiptID string `json:"receipt_id"`
	CheckedAt int64  `json:"checked_at"`
}
type SupportEligibility interface {
	SupportEligibility(context.Context, string) (Eligibility, error)
}
type PublishedPolicy struct {
	Document       string
	Language       string
	SourceLanguage string
	Text           string
	Revision       string
	Configured     bool
}
type PolicyReader interface {
	PublishedPolicy(context.Context, string, string, string) (PublishedPolicy, error)
}
type PublicReader interface {
	ServiceFacts(context.Context, string) (json.RawMessage, error)
	SetupGuide(context.Context, string, json.RawMessage) (json.RawMessage, error)
}
type Dependencies struct {
	SupportEligibility SupportEligibility
	Policies           PolicyReader
	Public             PublicReader
	Search             SearchProvider
	Navigation         NavigationReader
	PolicyChanges      PolicyChangePreparer
	Confirmations      ConfirmationAuthority
}

// New opens no connection and runs no DDL. It rejects installation owners and
// checks the already installed tables and precise runtime privileges.
func New(ctx context.Context, db *sql.DB, auth access.Authority, dependencies Dependencies) (*Service, error) {
	if db == nil || access.Missing(auth) {
		return nil, access.ErrInvalid
	}
	var unsafe bool
	var version int
	err := db.QueryRowContext(ctx, `SELECT current_user<>session_user OR EXISTS(SELECT FROM pg_roles WHERE rolname=current_user AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolbypassrls OR rolreplication)) OR has_database_privilege(current_database(),'CREATE,TEMP') OR EXISTS(SELECT FROM pg_namespace WHERE nspname LIKE 'core\_%' ESCAPE '\') OR EXISTS(SELECT FROM pg_namespace WHERE nspname NOT IN('pg_catalog','information_schema') AND nspname NOT LIKE 'pg\_%' ESCAPE '\' AND has_schema_privilege(oid,'CREATE')) OR EXISTS(SELECT FROM pg_roles WHERE rolname<>current_user AND (pg_has_role(current_user,oid,'USAGE') OR pg_has_role(current_user,oid,'SET') OR pg_has_role(current_user,oid,'MEMBER WITH ADMIN OPTION')))`).Scan(&unsafe)
	if err != nil {
		return nil, access.ErrUnavailable
	}
	if unsafe {
		return nil, access.ErrForbidden
	}
	if db.QueryRowContext(ctx, `SELECT version FROM lmm_assistant.services_contract WHERE singleton`).Scan(&version) != nil || version != 1 {
		return nil, access.ErrUnavailable
	}
	for _, table := range []string{"memories", "profiles", "service_operations", "service_network_dispatch", "service_confirmations", "site_issues", "site_issue_events", "platform_support", "platform_support_messages", "platform_support_todos"} {
		privileges := []string{"SELECT", "INSERT"}
		switch table {
		case "memories", "profiles":
			privileges = append(privileges, "UPDATE", "DELETE")
		case "site_issues", "platform_support", "platform_support_todos":
			privileges = append(privileges, "UPDATE")
		}
		for _, privilege := range privileges {
			var ok bool
			if db.QueryRowContext(ctx, `SELECT has_table_privilege('lmm_assistant.'||$1,$2)`, table, privilege).Scan(&ok) != nil || !ok {
				return nil, access.ErrUnavailable
			}
		}
		var forbidden bool
		if db.QueryRowContext(ctx, `SELECT has_table_privilege('lmm_assistant.'||$1,'TRUNCATE,TRIGGER,REFERENCES,MAINTAIN') OR ($1 IN('service_operations','service_network_dispatch','site_issue_events','platform_support_messages') AND (has_table_privilege('lmm_assistant.'||$1,'UPDATE,DELETE') OR has_any_column_privilege('lmm_assistant.'||$1,'UPDATE,REFERENCES')))`, table).Scan(&forbidden) != nil {
			return nil, access.ErrUnavailable
		}
		if forbidden {
			return nil, access.ErrForbidden
		}
	}
	for _, column := range []string{"consumed_at", "result"} {
		var ok bool
		if db.QueryRowContext(ctx, `SELECT has_column_privilege('lmm_assistant.service_confirmations',$1,'UPDATE')`, column).Scan(&ok) != nil || !ok {
			return nil, access.ErrUnavailable
		}
	}
	var forbidden bool
	if db.QueryRowContext(ctx, `SELECT has_table_privilege('lmm_assistant.service_confirmations','DELETE') OR EXISTS(SELECT FROM pg_attribute a WHERE a.attrelid='lmm_assistant.service_confirmations'::regclass AND a.attnum>0 AND NOT a.attisdropped AND a.attname NOT IN('consumed_at','result') AND has_column_privilege(a.attrelid,a.attname,'UPDATE'))`).Scan(&forbidden) != nil {
		return nil, access.ErrUnavailable
	}
	if forbidden {
		return nil, access.ErrForbidden
	}
	for _, name := range []string{"memories", "site_issues", "site_issue_events", "platform_support", "platform_support_messages"} {
		var ok bool
		if db.QueryRowContext(ctx, `SELECT has_sequence_privilege('lmm_assistant.'||$1||'_id_seq','USAGE') AND NOT has_sequence_privilege('lmm_assistant.'||$1||'_id_seq','UPDATE')`, name).Scan(&ok) != nil || !ok {
			return nil, access.ErrForbidden
		}
	}
	out := &Service{db: db, auth: auth, dependencies: dependencies, now: time.Now}
	out.confirmations = out.defaultConfirmations()
	return out, nil
}
func decode(raw json.RawMessage, out any) error { return access.Decode(raw, out) }
func digest(raw []byte) string                  { sum := sha256.Sum256(raw); return hex.EncodeToString(sum[:]) }
func result(operation string, state assistant.AgentToolState, value any, receipt string) (assistant.AgentToolResult, error) {
	raw, err := json.Marshal(value)
	if err != nil || len(raw) > 64<<10 {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	return assistant.AgentToolResult{OperationID: operation, State: state, ResultJSON: raw, ResultSHA256: digest(raw), DomainReceiptID: receipt}, nil
}
func (s *Service) principal(ctx context.Context, credential string) (access.Principal, error) {
	p, err := s.auth.Check(ctx, credential, access.Account{}, access.Read)
	if err != nil {
		return p, err
	}
	if p.UserID <= 0 || p.Account != (access.Account{Kind: "personal", ID: p.UserID}) {
		return p, access.ErrForbidden
	}
	return p, nil
}
func (s *Service) validate(ctx context.Context, in assistant.AgentInvocation) (access.Principal, [32]byte, [32]byte, error) {
	p, err := s.principal(ctx, in.Credential)
	if err != nil {
		return p, [32]byte{}, [32]byte{}, err
	}
	if p.UserID != in.Actor.UserID || p.Account != in.Actor.Account || len(in.Call.OperationID) < 16 || len(in.Call.OperationID) > 128 || len(in.Call.CoreConversationID) < 1 || len(in.Call.CoreConversationID) > 128 || in.Call.ArgumentsSHA256 != digest(in.Call.Arguments) || in.Call.Scope.Version != assistant.FixedMainToolVersion || in.Call.Scope.PriceVersion != assistant.BuiltinToolPriceVersion || in.Call.Scope.PriceSHA256 != assistant.FreeAgentPriceSHA256() || len(in.Call.Scope.SchemaSHA256) != 64 {
		return p, [32]byte{}, [32]byte{}, access.ErrInvalid
	}
	raw, err := json.Marshal(in.Call)
	if err != nil {
		return p, [32]byte{}, [32]byte{}, access.ErrInvalid
	}
	return p, sha256.Sum256([]byte(in.Credential)), sha256.Sum256(raw), nil
}
func (s *Service) readOperation(ctx context.Context, tx *sql.Tx, in assistant.AgentInvocation, p access.Principal, session, call [32]byte) (assistant.AgentToolResult, error) {
	var raw []byte
	var user int64
	var savedSession, savedCall []byte
	err := tx.QueryRowContext(ctx, `SELECT user_id,session_sha256,call_sha256,result_bytes FROM lmm_assistant.service_operations WHERE operation_id=$1`, in.Call.OperationID).Scan(&user, &savedSession, &savedCall, &raw)
	if errors.Is(err, sql.ErrNoRows) {
		return assistant.AgentToolResult{}, access.ErrNotFound
	}
	if err != nil {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	if user != p.UserID || !bytes.Equal(savedSession, session[:]) || !bytes.Equal(savedCall, call[:]) {
		return assistant.AgentToolResult{}, access.ErrConflict
	}
	var out assistant.AgentToolResult
	if json.Unmarshal(raw, &out) != nil || out.OperationID != in.Call.OperationID || out.ResultSHA256 != digest(out.ResultJSON) {
		return out, access.ErrUnavailable
	}
	if out.State == assistant.AgentToolPending {
		var completed []byte
		var receipt string
		err = tx.QueryRowContext(ctx, `SELECT id,result FROM lmm_assistant.service_confirmations WHERE operation_id=$1 AND consumed_at IS NOT NULL`, in.Call.OperationID).Scan(&receipt, &completed)
		if err == nil {
			var value map[string]any
			if json.Unmarshal(completed, &value) != nil {
				return out, access.ErrUnavailable
			}
			return result(in.Call.OperationID, assistant.AgentToolSucceeded, value, receipt)
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return out, access.ErrUnavailable
		}
	}
	return out, nil
}
func (s *Service) Inspect(ctx context.Context, in assistant.AgentInvocation) (assistant.AgentToolResult, error) {
	p, session, call, err := s.validate(ctx, in)
	if err != nil {
		return assistant.AgentToolResult{}, err
	}
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	defer tx.Rollback()
	out, err := s.readOperation(ctx, tx, in, p, session, call)
	if errors.Is(err, access.ErrNotFound) {
		var user int64
		var savedSession, savedCall []byte
		markerErr := tx.QueryRowContext(ctx, `SELECT user_id,session_sha256,call_sha256 FROM lmm_assistant.service_network_dispatch WHERE operation_id=$1`, in.Call.OperationID).Scan(&user, &savedSession, &savedCall)
		if markerErr == nil && (user != p.UserID || !bytes.Equal(session[:], savedSession) || !bytes.Equal(call[:], savedCall)) {
			return assistant.AgentToolResult{}, access.ErrConflict
		}
		if markerErr != nil && !errors.Is(markerErr, sql.ErrNoRows) {
			return assistant.AgentToolResult{}, access.ErrUnavailable
		}
		return unknownResult(in.Call.OperationID), nil
	}
	return out, err
}
func (s *Service) Execute(ctx context.Context, in assistant.AgentInvocation) (assistant.AgentToolResult, error) {
	if in.Call.Scope.Name == "search_web" {
		return s.search(ctx, in)
	}
	return s.execute(ctx, in, nil)
}
func (s *Service) execute(ctx context.Context, in assistant.AgentInvocation, payload *json.RawMessage) (assistant.AgentToolResult, error) {
	p, session, call, err := s.validate(ctx, in)
	if err != nil {
		return assistant.AgentToolResult{}, err
	}
	bounded, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	tx, err := s.db.BeginTx(bounded, &sql.TxOptions{})
	if err != nil {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(bounded, `SELECT pg_advisory_xact_lock(hashtextextended($1,0))`, "assistant-services:"+stringID(p.UserID)); err != nil {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	prior, err := s.readOperation(bounded, tx, in, p, session, call)
	if err == nil {
		return prior, nil
	}
	if !errors.Is(err, access.ErrNotFound) {
		return assistant.AgentToolResult{}, err
	}
	var out assistant.AgentToolResult
	if payload != nil {
		out, err = s.prepareInTx(bounded, tx, in, p, session, *payload)
	} else {
		out, err = s.dispatch(bounded, tx, in, p, session)
	}
	if err != nil {
		return assistant.AgentToolResult{}, err
	}
	raw, err := json.Marshal(out)
	if err != nil {
		return out, access.ErrUnavailable
	}
	if _, err = tx.ExecContext(bounded, `INSERT INTO lmm_assistant.service_operations(operation_id,user_id,session_sha256,call_sha256,tool_name,result,result_bytes,created_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8)`, in.Call.OperationID, p.UserID, session[:], call[:], in.Call.Scope.Name, string(raw), raw, s.now().Unix()); err != nil {
		return out, access.ErrUnavailable
	}
	if tx.Commit() != nil {
		return assistant.AgentToolResult{OperationID: in.Call.OperationID, State: assistant.AgentToolUnknown}, access.ErrUnavailable
	}
	return out, nil
}
func stringID(id int64) string {
	var value [20]byte
	i := len(value)
	for id > 0 {
		i--
		value[i] = byte('0' + id%10)
		id /= 10
	}
	return string(value[i:])
}
func newID() (string, error) {
	var raw [32]byte
	if _, err := rand.Read(raw[:]); err != nil {
		return "", access.ErrUnavailable
	}
	return hex.EncodeToString(raw[:]), nil
}
func safeText(value string, max int) string {
	value = strings.TrimSpace(value)
	runes := []rune(value)
	if len(runes) > max {
		value = string(runes[:max])
	}
	return value
}
