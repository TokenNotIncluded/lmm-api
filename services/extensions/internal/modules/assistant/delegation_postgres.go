package assistant

import (
	"bytes"
	"context"
	"database/sql"
	_ "embed"
	"encoding/json"
	"errors"
	"strconv"
	"time"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/access"
)

// DelegationSchema is an explicit, separately reviewed module migration. Neither
// the constructor nor runtime startup installs it or changes contract v2.
//
//go:embed delegation_schema.sql
var DelegationSchema string

type DelegationPostgresJournal struct{ db *sql.DB }

func NewDelegationPostgresJournal(ctx context.Context, db *sql.DB, expectedDatabase string) (*DelegationPostgresJournal, error) {
	if db == nil || expectedDatabase == "" {
		return nil, access.ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	var database string
	var hasCore bool
	var historyVersion, delegationVersion int
	if db.QueryRowContext(ctx, `SELECT current_database(),EXISTS(SELECT 1 FROM pg_namespace WHERE nspname LIKE 'core\_%' ESCAPE '\')`).Scan(&database, &hasCore) != nil {
		return nil, access.ErrUnavailable
	}
	if database != expectedDatabase || hasCore {
		return nil, access.ErrForbidden
	}
	if db.QueryRowContext(ctx, `SELECT version FROM lmm_assistant.contract WHERE singleton`).Scan(&historyVersion) != nil || historyVersion != 2 ||
		db.QueryRowContext(ctx, `SELECT version FROM lmm_assistant.delegation_contract WHERE singleton`).Scan(&delegationVersion) != nil || delegationVersion != 1 {
		return nil, ErrDelegationUnavailable
	}
	return &DelegationPostgresJournal{db: db}, nil
}

func (p *DelegationPostgresJournal) Within(ctx context.Context, userID, conversationID int64, turnID string, fn func(*DelegationLocalState) error) error {
	if userID <= 0 || conversationID <= 0 || conversationID > maxSafeHistoryID || !delegationLabel(turnID, 124) || fn == nil {
		return access.ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	tx, err := p.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return access.ErrUnavailable
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, `SELECT pg_advisory_xact_lock(hashtextextended($1,0))`, "lmm_assistant_delegation:"+strconv.FormatInt(conversationID, 10)); err != nil {
		return access.ErrUnavailable
	}
	// This row, not a Go/browser owner flag, establishes local provenance. Core
	// separately owns and checks the random Core conversation handle and Session.
	var archivedAt, restrictedAt int64
	if err = tx.QueryRowContext(ctx, `SELECT archived_at,restricted_at FROM lmm_assistant.conversations WHERE id=$1 AND user_id=$2 FOR UPDATE`, conversationID, userID).Scan(&archivedAt, &restrictedAt); err != nil {
		return historyError(err)
	}
	state := DelegationLocalState{CanStart: archivedAt == 0 && restrictedAt == 0}
	var creationKey, coreID, phase string
	err = tx.QueryRowContext(ctx, `SELECT creation_key,COALESCE(core_conversation_id,''),phase FROM lmm_assistant.delegation_conversations WHERE user_id=$1 AND conversation_id=$2`, userID, conversationID).Scan(&creationKey, &coreID, &phase)
	if err == nil {
		state.Mapping = &DelegationConversationMapping{CreationKey: creationKey, CoreConversationID: coreID, Phase: phase}
	} else if !errors.Is(err, sql.ErrNoRows) {
		return access.ErrUnavailable
	}
	var priorBody []byte
	var priorConversation int64
	err = tx.QueryRowContext(ctx, `SELECT conversation_id,body FROM lmm_assistant.delegation_turns WHERE user_id=$1 AND turn_id=$2`, userID, turnID).Scan(&priorConversation, &priorBody)
	if err == nil {
		if priorConversation != conversationID || json.Unmarshal(priorBody, &state.Turn) != nil || !validStoredDelegationTurn(state.Turn) || state.Turn.ID != turnID {
			return access.ErrConflict
		}
	} else if !errors.Is(err, sql.ErrNoRows) {
		return access.ErrUnavailable
	}
	beforeMapping, _ := json.Marshal(state.Mapping)
	beforeTurn, _ := json.Marshal(state.Turn)
	var originalMapping *DelegationConversationMapping
	if state.Mapping != nil {
		copy := *state.Mapping
		originalMapping = &copy
	}
	if err = fn(&state); err != nil {
		return err
	}
	if state.Mapping == nil && string(beforeMapping) != "null" || state.Turn == nil && string(beforeTurn) != "null" {
		return access.ErrConflict
	}
	afterMapping, err := json.Marshal(state.Mapping)
	if err != nil {
		return access.ErrInvalid
	}
	if !bytes.Equal(beforeMapping, afterMapping) {
		m := state.Mapping
		if m == nil || !delegationLabel(m.CreationKey, 128) || m.Phase != "creation_inflight" && m.Phase != "registered" ||
			m.Phase == "registered" && !coreConversationID(m.CoreConversationID) || m.Phase == "creation_inflight" && m.CoreConversationID != "" {
			return access.ErrInvalid
		}
		if originalMapping != nil && (originalMapping.CreationKey != m.CreationKey || originalMapping.CoreConversationID != "" && originalMapping.CoreConversationID != m.CoreConversationID) {
			return access.ErrConflict
		}
		result, err := tx.ExecContext(ctx, `INSERT INTO lmm_assistant.delegation_conversations(user_id,conversation_id,creation_key,core_conversation_id,phase) VALUES($1,$2,$3,NULLIF($4,''),$5) ON CONFLICT(user_id,conversation_id) DO UPDATE SET core_conversation_id=EXCLUDED.core_conversation_id,phase=EXCLUDED.phase WHERE lmm_assistant.delegation_conversations.creation_key=EXCLUDED.creation_key`, userID, conversationID, m.CreationKey, m.CoreConversationID, m.Phase)
		if err != nil {
			return access.ErrUnavailable
		}
		if count, err := result.RowsAffected(); err != nil || count != 1 {
			return access.ErrConflict
		}
	}
	afterTurn, err := json.Marshal(state.Turn)
	if err != nil || len(afterTurn) > 1<<20 {
		return access.ErrLimit
	}
	if !bytes.Equal(beforeTurn, afterTurn) {
		if !validStoredDelegationTurn(state.Turn) || state.Turn.ID != turnID || state.Turn.Grant != nil && (state.Mapping == nil || state.Turn.Grant.CoreConversationID != state.Mapping.CoreConversationID) {
			return access.ErrInvalid
		}
		var previous *DelegationStoredTurn
		if json.Unmarshal(beforeTurn, &previous) != nil {
			return access.ErrUnavailable
		}
		if !validStoredDelegationUpdate(previous, state.Turn) {
			return access.ErrConflict
		}
		result, err := tx.ExecContext(ctx, `INSERT INTO lmm_assistant.delegation_turns(user_id,conversation_id,turn_id,body) VALUES($1,$2,$3,$4::jsonb) ON CONFLICT(user_id,turn_id) DO UPDATE SET body=EXCLUDED.body,revision=lmm_assistant.delegation_turns.revision+1 WHERE lmm_assistant.delegation_turns.conversation_id=EXCLUDED.conversation_id`, userID, conversationID, turnID, string(afterTurn))
		if err != nil {
			return access.ErrUnavailable
		}
		if count, err := result.RowsAffected(); err != nil || count != 1 {
			return access.ErrConflict
		}
	}
	if tx.Commit() != nil {
		// Never send an Issue/Invoke after a possibly committed claim returned
		// an unknown ACK. A subsequent read recovers the ORIGINAL journal row.
		return ErrDelegationUnknown
	}
	return nil
}

func coreConversationID(value string) bool {
	return len(value) == len("lmm_cv_")+64 && value[:len("lmm_cv_")] == "lmm_cv_" && lowerDigest(value[len("lmm_cv_"):])
}
