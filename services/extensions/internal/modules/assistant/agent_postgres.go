package assistant

import (
	"context"
	"database/sql"
	_ "embed"
	"encoding/json"
	"errors"
	"strconv"
	"time"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/access"
)

// AgentSchema is an explicit reviewed install/upgrade input, never startup DDL.
//
//go:embed agent_schema.sql
var AgentSchema string

type AgentPostgresJournal struct{ db *sql.DB }

// encoding/json compacts RawMessage when nesting it. Base64 byte slices retain
// the original provider argument and domain result bytes beside the readable
// state; these exact bytes, rather than a reserialization, define their hashes.
type agentDiskTurn struct {
	State            *AgentStoredTurn  `json:"state"`
	Arguments        map[string][]byte `json:"arguments"`
	Results          map[string][]byte `json:"results"`
	PendingArguments map[string][]byte `json:"pending_arguments"`
}

func encodeAgentDiskTurn(turn *AgentStoredTurn) ([]byte, error) {
	disk := agentDiskTurn{State: turn, Arguments: map[string][]byte{}, Results: map[string][]byte{}, PendingArguments: map[string][]byte{}}
	for id, tool := range turn.Tools {
		disk.Arguments[id] = tool.Call.Arguments
		if tool.Result != nil {
			disk.Results[id] = tool.Result.ResultJSON
		}
	}
	if turn.Snapshot != nil {
		for _, call := range turn.Snapshot.PendingTools {
			disk.PendingArguments[call.OperationID] = call.Arguments
		}
	}
	return json.Marshal(disk)
}
func decodeAgentDiskTurn(raw []byte) (*AgentStoredTurn, error) {
	var disk agentDiskTurn
	if json.Unmarshal(raw, &disk) != nil || disk.State == nil {
		return nil, access.ErrUnavailable
	}
	for id, tool := range disk.State.Tools {
		if tool == nil {
			return nil, access.ErrUnavailable
		}
		args, ok := disk.Arguments[id]
		if !ok {
			return nil, access.ErrUnavailable
		}
		tool.Call.Arguments = args
		if tool.Result != nil {
			result, ok := disk.Results[id]
			if !ok {
				return nil, access.ErrUnavailable
			}
			tool.Result.ResultJSON = result
		}
	}
	if disk.State.Snapshot != nil {
		for i := range disk.State.Snapshot.PendingTools {
			call := &disk.State.Snapshot.PendingTools[i]
			args, ok := disk.PendingArguments[call.OperationID]
			if !ok {
				return nil, access.ErrUnavailable
			}
			call.Arguments = args
		}
	}
	return disk.State, nil
}

var _ AgentJournal = (*AgentPostgresJournal)(nil)

func NewAgentPostgresJournal(ctx context.Context, db *sql.DB, expectedDatabase string) (*AgentPostgresJournal, error) {
	if db == nil || expectedDatabase == "" {
		return nil, access.ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	var database string
	var hasCore bool
	if db.QueryRowContext(ctx, `SELECT current_database(),EXISTS(SELECT 1 FROM pg_namespace WHERE nspname LIKE 'core\_%' ESCAPE '\')`).Scan(&database, &hasCore) != nil {
		return nil, access.ErrUnavailable
	}
	if database != expectedDatabase || hasCore {
		return nil, access.ErrForbidden
	}
	for _, contract := range []struct {
		query    string
		expected int
	}{{`SELECT version FROM lmm_assistant.contract WHERE singleton`, 2}, {`SELECT version FROM lmm_assistant.delegation_contract WHERE singleton`, 1}, {`SELECT version FROM lmm_assistant.agent_contract WHERE singleton`, 1}} {
		var value int
		if db.QueryRowContext(ctx, contract.query).Scan(&value) != nil || value != contract.expected {
			return nil, ErrDelegationUnavailable
		}
	}
	var exactBytes bool
	if db.QueryRowContext(ctx, `SELECT EXISTS(SELECT 1 FROM pg_attribute WHERE attrelid='lmm_assistant.agent_turns'::regclass AND attname='body_bytes' AND atttypid='bytea'::regtype AND attnotnull AND NOT attisdropped)`).Scan(&exactBytes) != nil || !exactBytes {
		return nil, ErrDelegationUnavailable
	}
	return &AgentPostgresJournal{db: db}, nil
}

func (p *AgentPostgresJournal) WithinAgent(ctx context.Context, owner, conversation int64, turnID string, fn func(*AgentLocalState) error) error {
	if owner <= 0 || conversation <= 0 || conversation > maxSafeHistoryID || !access.ID(turnID) || len(turnID) > 64 || fn == nil {
		return access.ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	tx, err := p.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return access.ErrUnavailable
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, `SELECT pg_advisory_xact_lock(hashtextextended($1,0))`, "lmm_assistant_delegation:"+strconv.FormatInt(conversation, 10)); err != nil {
		return access.ErrUnavailable
	}
	var archived, restricted int64
	if err = tx.QueryRowContext(ctx, `SELECT archived_at,restricted_at FROM lmm_assistant.conversations WHERE id=$1 AND user_id=$2 FOR UPDATE`, conversation, owner).Scan(&archived, &restricted); err != nil {
		return historyError(err)
	}
	state := AgentLocalState{CanStart: archived == 0 && restricted == 0}
	var mapping DelegationConversationMapping
	err = tx.QueryRowContext(ctx, `SELECT creation_key,COALESCE(core_conversation_id,''),phase FROM lmm_assistant.delegation_conversations WHERE user_id=$1 AND conversation_id=$2`, owner, conversation).Scan(&mapping.CreationKey, &mapping.CoreConversationID, &mapping.Phase)
	if err == nil {
		state.Mapping = &mapping
	} else if !errors.Is(err, sql.ErrNoRows) {
		return access.ErrUnavailable
	}
	var priorBody []byte
	var priorConversation int64
	// JSONB normalizes nested RawMessage bytes. Keep the exact immutable wire
	// bytes separately so schema/argument/result digests survive a DB restart.
	err = tx.QueryRowContext(ctx, `SELECT conversation_id,body_bytes FROM lmm_assistant.agent_turns WHERE user_id=$1 AND turn_id=$2`, owner, turnID).Scan(&priorConversation, &priorBody)
	if err == nil {
		state.Turn, err = decodeAgentDiskTurn(priorBody)
		if priorConversation != conversation || len(priorBody) > 32<<20 || err != nil || !validAgentStoredTurn(state.Turn) || state.Turn.ID != turnID {
			return access.ErrConflict
		}
	} else if !errors.Is(err, sql.ErrNoRows) {
		return access.ErrUnavailable
	}
	var beforeMapping *DelegationConversationMapping
	if state.Mapping != nil {
		copy := *state.Mapping
		beforeMapping = &copy
	}
	var beforeTurn *AgentStoredTurn
	if len(priorBody) > 0 {
		beforeTurn, err = decodeAgentDiskTurn(priorBody)
		if err != nil {
			return err
		}
	}
	if err = fn(&state); err != nil {
		return err
	}
	if state.Mapping == nil && beforeMapping != nil || state.Turn == nil && beforeTurn != nil {
		return access.ErrConflict
	}
	if state.Mapping != nil {
		m := state.Mapping
		if !delegationLabel(m.CreationKey, 128) || m.Phase != "creation_inflight" && m.Phase != "registered" || m.Phase == "registered" && !coreConversationID(m.CoreConversationID) || m.Phase == "creation_inflight" && m.CoreConversationID != "" {
			return access.ErrInvalid
		}
		if beforeMapping != nil && (beforeMapping.CreationKey != m.CreationKey || beforeMapping.CoreConversationID != "" && beforeMapping.CoreConversationID != m.CoreConversationID || beforeMapping.Phase == "registered" && m.Phase != "registered") {
			return access.ErrConflict
		}
		if !sameAgentJSON(beforeMapping, m) {
			result, e := tx.ExecContext(ctx, `INSERT INTO lmm_assistant.delegation_conversations(user_id,conversation_id,creation_key,core_conversation_id,phase) VALUES($1,$2,$3,NULLIF($4,''),$5) ON CONFLICT(user_id,conversation_id) DO UPDATE SET core_conversation_id=EXCLUDED.core_conversation_id,phase=EXCLUDED.phase WHERE lmm_assistant.delegation_conversations.creation_key=EXCLUDED.creation_key`, owner, conversation, m.CreationKey, m.CoreConversationID, m.Phase)
			if e != nil {
				return access.ErrUnavailable
			}
			if n, e := result.RowsAffected(); e != nil || n != 1 {
				return access.ErrConflict
			}
		}
	}
	if state.Turn != nil {
		if !validAgentStoredUpdate(beforeTurn, state.Turn) || state.Turn.ID != turnID {
			return access.ErrConflict
		}
		if state.Turn.Snapshot != nil && (state.Mapping == nil || state.Mapping.CoreConversationID != state.Turn.Snapshot.Key.CoreConversationID || state.Turn.Snapshot.OwnerUserID != owner) {
			return access.ErrConflict
		}
		if !sameAgentJSON(beforeTurn, state.Turn) {
			body, e := json.Marshal(state.Turn)
			if e != nil || len(body) > 32<<20 {
				return access.ErrLimit
			}
			disk, e := encodeAgentDiskTurn(state.Turn)
			if e != nil || len(disk) > 32<<20 {
				return access.ErrLimit
			}
			_, e = tx.ExecContext(ctx, `INSERT INTO lmm_assistant.agent_turns(user_id,conversation_id,turn_id,body,body_bytes) VALUES($1,$2,$3,$4::jsonb,$5) ON CONFLICT(user_id,turn_id) DO UPDATE SET body=EXCLUDED.body,body_bytes=EXCLUDED.body_bytes,revision=lmm_assistant.agent_turns.revision+1`, owner, conversation, turnID, string(body), disk)
			if e != nil {
				return access.ErrUnavailable
			}
		}
	}
	if tx.Commit() != nil {
		return ErrDelegationUnknown
	}
	return nil
}
