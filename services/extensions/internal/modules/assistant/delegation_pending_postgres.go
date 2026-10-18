package assistant

import (
	"context"
	"database/sql"
	"errors"
	"strconv"
	"time"

	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant/access"
)

var _ PendingConversationRepository = (*DelegationPostgresJournal)(nil)

const pendingConversationSelect = `SELECT c.id,m.creation_key,COALESCE(m.core_conversation_id,''),m.phase
FROM lmm_assistant.conversations c JOIN lmm_assistant.delegation_conversations m
ON m.user_id=c.user_id AND m.conversation_id=c.id WHERE c.user_id=$1 AND m.creation_key=$2`

func scanPending(row rowScanner) (PendingConversation, error) {
	var p PendingConversation
	if err := row.Scan(&p.ID, &p.CreationKey, &p.CoreConversationID, &p.Phase); err != nil {
		return PendingConversation{}, err
	}
	if !validPendingConversation(p) {
		return PendingConversation{}, access.ErrUnavailable
	}
	return p, nil
}

func (p *DelegationPostgresJournal) CreatePending(ctx context.Context, owner int64, key string, now int64) (PendingConversation, bool, error) {
	if owner <= 0 || !pendingCreationKey(key) || now <= 0 {
		return PendingConversation{}, false, access.ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	tx, err := p.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return PendingConversation{}, false, access.ErrUnavailable
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, `SELECT pg_advisory_xact_lock(hashtextextended($1,0))`, "lmm_assistant_pending:"+strconv.FormatInt(owner, 10)+":"+key); err != nil {
		return PendingConversation{}, false, access.ErrUnavailable
	}
	local, err := scanPending(tx.QueryRowContext(ctx, pendingConversationSelect+` FOR UPDATE OF c,m`, owner, key))
	fresh := false
	if errors.Is(err, sql.ErrNoRows) {
		// The old schema requires a nonempty preview, so this is an explicit
		// pending label. No message or completed turn receipt is inserted.
		var id int64
		err = tx.QueryRowContext(ctx, `INSERT INTO lmm_assistant.conversations(user_id,title,last_message_preview,created_at,updated_at) VALUES($1,'New conversation','Pending',$2,$2) RETURNING id`, owner, now).Scan(&id)
		if err == nil {
			_, err = tx.ExecContext(ctx, `INSERT INTO lmm_assistant.delegation_conversations(user_id,conversation_id,creation_key,phase) VALUES($1,$2,$3,'creation_inflight')`, owner, id, key)
		}
		if err != nil {
			return PendingConversation{}, false, access.ErrUnavailable
		}
		local = PendingConversation{ID: id, CreationKey: key, Phase: "creation_inflight"}
		fresh = true
	} else if err != nil {
		return PendingConversation{}, false, historyError(err)
	}
	if tx.Commit() != nil {
		return PendingConversation{}, false, ErrDelegationUnknown
	}
	return local, fresh, nil
}

func (p *DelegationPostgresJournal) LookupPending(ctx context.Context, owner int64, key string) (PendingConversation, error) {
	if owner <= 0 || !pendingCreationKey(key) {
		return PendingConversation{}, access.ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	local, err := scanPending(p.db.QueryRowContext(ctx, pendingConversationSelect, owner, key))
	if err != nil {
		return PendingConversation{}, historyError(err)
	}
	return local, nil
}

func (p *DelegationPostgresJournal) BindPending(ctx context.Context, owner int64, key, coreID string) (PendingConversation, error) {
	if owner <= 0 || !pendingCreationKey(key) || !coreConversationID(coreID) {
		return PendingConversation{}, access.ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	tx, err := p.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return PendingConversation{}, access.ErrUnavailable
	}
	defer tx.Rollback()
	local, err := scanPending(tx.QueryRowContext(ctx, pendingConversationSelect+` FOR UPDATE OF c,m`, owner, key))
	if err != nil {
		return PendingConversation{}, historyError(err)
	}
	if local.CoreConversationID != "" && local.CoreConversationID != coreID {
		return PendingConversation{}, access.ErrConflict
	}
	if _, err = tx.ExecContext(ctx, `UPDATE lmm_assistant.delegation_conversations SET core_conversation_id=$3,phase='registered' WHERE user_id=$1 AND creation_key=$2`, owner, key, coreID); err != nil {
		return PendingConversation{}, access.ErrUnavailable
	}
	if tx.Commit() != nil {
		return PendingConversation{}, ErrDelegationUnknown
	}
	local.CoreConversationID, local.Phase = coreID, "registered"
	return local, nil
}
