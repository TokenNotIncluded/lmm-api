package assistant_services

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/access"
	"strings"
	"unicode/utf8"
)

func unknownResult(operation string) assistant.AgentToolResult {
	out, _ := result(operation, assistant.AgentToolUnknown, map[string]any{"ok": false, "status": "unknown", "retry_safe": false}, "")
	return out
}

// search commits its dispatch claim before network I/O. A crash or ambiguous
// transport outcome remains inspectable UNKNOWN and can never resend this call.
func (s *Service) search(ctx context.Context, in assistant.AgentInvocation) (assistant.AgentToolResult, error) {
	p, session, call, err := s.validate(ctx, in)
	if err != nil {
		return assistant.AgentToolResult{}, err
	}
	var args struct {
		Query string `json:"query"`
	}
	if decode(in.Call.Arguments, &args) != nil {
		return assistant.AgentToolResult{}, access.ErrInvalid
	}
	args.Query = strings.TrimSpace(args.Query)
	if utf8.RuneCountInString(args.Query) < 2 || !utf8.ValidString(args.Query) || utf8.RuneCountInString(args.Query) > 500 {
		return assistant.AgentToolResult{}, access.ErrInvalid
	}
	if access.Missing(s.dependencies.Search) {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, `SELECT pg_advisory_xact_lock(hashtextextended($1,0))`, "assistant-services:"+stringID(p.UserID)); err != nil {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	prior, err := s.readOperation(ctx, tx, in, p, session, call)
	if err == nil {
		return prior, nil
	}
	if !errors.Is(err, access.ErrNotFound) {
		return assistant.AgentToolResult{}, err
	}
	var savedUser int64
	var savedSession, savedCall []byte
	err = tx.QueryRowContext(ctx, `SELECT user_id,session_sha256,call_sha256 FROM lmm_assistant.service_network_dispatch WHERE operation_id=$1`, in.Call.OperationID).Scan(&savedUser, &savedSession, &savedCall)
	if err == nil {
		if savedUser != p.UserID || !bytes.Equal(session[:], savedSession) || !bytes.Equal(call[:], savedCall) {
			return assistant.AgentToolResult{}, access.ErrConflict
		}
		return unknownResult(in.Call.OperationID), nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO lmm_assistant.service_network_dispatch(operation_id,user_id,session_sha256,call_sha256,created_at) VALUES($1,$2,$3,$4,$5)`, in.Call.OperationID, p.UserID, session[:], call[:], s.now().Unix()); err != nil {
		return assistant.AgentToolResult{}, access.ErrUnavailable
	}
	if tx.Commit() != nil {
		return unknownResult(in.Call.OperationID), access.ErrUnavailable
	}
	raw, err := s.dependencies.Search.Search(ctx, args.Query)
	if err != nil {
		var dispatch interface{ DispatchUnknown() bool }
		if !errors.As(err, &dispatch) || dispatch.DispatchUnknown() {
			return unknownResult(in.Call.OperationID), nil
		}
		raw = json.RawMessage(`{"ok":false,"status":"not_dispatched","retry_safe":false}`)
	}
	var value map[string]any
	if len(raw) > 64<<10 || json.Unmarshal(raw, &value) != nil || value == nil {
		return unknownResult(in.Call.OperationID), nil
	}
	state := assistant.AgentToolSucceeded
	if ok, exists := value["ok"].(bool); exists && !ok {
		state = assistant.AgentToolFailed
	}
	out, err := result(in.Call.OperationID, state, value, in.Call.OperationID)
	if err != nil {
		return unknownResult(in.Call.OperationID), err
	}
	// Session revocation after I/O cannot expose the result. The durable marker
	// still prevents another external request after credentials are restored.
	current, err := s.principal(ctx, in.Credential)
	if err != nil || current.UserID != p.UserID {
		return unknownResult(in.Call.OperationID), access.ErrForbidden
	}
	encoded, err := json.Marshal(out)
	if err != nil {
		return unknownResult(in.Call.OperationID), access.ErrUnavailable
	}
	if _, err = s.db.ExecContext(ctx, `INSERT INTO lmm_assistant.service_operations(operation_id,user_id,session_sha256,call_sha256,tool_name,result,result_bytes,created_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8)`, in.Call.OperationID, p.UserID, session[:], call[:], in.Call.Scope.Name, string(encoded), encoded, s.now().Unix()); err != nil {
		return unknownResult(in.Call.OperationID), access.ErrUnavailable
	}
	return out, nil
}
