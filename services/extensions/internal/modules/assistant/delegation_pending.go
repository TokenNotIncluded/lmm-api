package assistant

import (
	"context"
	"errors"
	"time"

	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant/access"
)

// PendingConversation is a real empty local conversation with no completed
// messages/receipt. Its phase is persisted in delegation_conversations. Only
// Core's authenticated Begin can supply CoreConversationID.
type PendingConversation struct {
	ID                 int64  `json:"id"`
	CreationKey        string `json:"creation_key"`
	CoreConversationID string `json:"core_conversation_id,omitempty"`
	Phase              string `json:"phase"`
}

type PendingConversationRepository interface {
	CreatePending(context.Context, int64, string, int64) (PendingConversation, bool, error)
	LookupPending(context.Context, int64, string) (PendingConversation, error)
	BindPending(context.Context, int64, string, string) (PendingConversation, error)
}

type PendingConversations struct {
	authority access.Authority
	repo      PendingConversationRepository
	port      DurableDelegationPort
	now       func() time.Time
}

func NewPendingConversations(authority access.Authority, repo PendingConversationRepository, port DurableDelegationPort) (*PendingConversations, error) {
	if access.Missing(authority) || access.Missing(repo) || access.Missing(port) {
		return nil, ErrDelegationUnavailable
	}
	return &PendingConversations{authority: authority, repo: repo, port: port, now: time.Now}, nil
}

// A caller keeps this opaque idempotency key across UNKNOWN outcomes. It does
// not identify an owner, authenticate a Session or authorize Core spending.
func pendingCreationKey(key string) bool {
	if len(key) != 32 {
		return false
	}
	for _, b := range []byte(key) {
		if !(b >= '0' && b <= '9' || b >= 'a' && b <= 'f') {
			return false
		}
	}
	return true
}

func (p *PendingConversations) owner(ctx context.Context, credential string) (int64, error) {
	principal, err := p.authority.Check(ctx, credential, access.Account{}, access.Read)
	if err != nil {
		return 0, err
	}
	if principal.UserID <= 0 || principal.Level < 0 || principal.Level > 6 || principal.Account != (access.Account{Kind: "personal", ID: principal.UserID}) {
		return 0, access.ErrForbidden
	}
	return principal.UserID, nil
}

// Start commits the empty local row and creation claim before any Core call.
// An existing key is ONLY inspected; no retry can send another Begin. A local
// commit ACK failure returns UNKNOWN and sends no RPC.
func (p *PendingConversations) Start(ctx context.Context, credential, creationKey string) (PendingConversation, error) {
	if !pendingCreationKey(creationKey) {
		return PendingConversation{}, access.ErrInvalid
	}
	owner, err := p.owner(ctx, credential)
	if err != nil {
		return PendingConversation{}, err
	}
	local, fresh, err := p.repo.CreatePending(ctx, owner, creationKey, p.now().Unix())
	if err != nil {
		return PendingConversation{}, err
	}
	return p.confirm(ctx, credential, owner, local, fresh)
}

// Recover is read-only until a valid receipt is copied into the local mapping.
// Missing local/Core receipts remain UNKNOWN. It never creates a conversation,
// calls Begin, changes creation key, issues a grant or fabricates model history.
func (p *PendingConversations) Recover(ctx context.Context, credential, creationKey string) (PendingConversation, error) {
	if !pendingCreationKey(creationKey) {
		return PendingConversation{}, access.ErrInvalid
	}
	owner, err := p.owner(ctx, credential)
	if err != nil {
		return PendingConversation{}, err
	}
	local, err := p.repo.LookupPending(ctx, owner, creationKey)
	if err != nil {
		if errors.Is(err, access.ErrNotFound) {
			err = ErrDelegationUnknown
		}
		return PendingConversation{}, err
	}
	return p.confirm(ctx, credential, owner, local, false)
}

func (p *PendingConversations) confirm(ctx context.Context, credential string, owner int64, local PendingConversation, fresh bool) (PendingConversation, error) {
	if !validPendingConversation(local) {
		return PendingConversation{}, access.ErrUnavailable
	}
	if fresh && local.Phase != "creation_inflight" {
		return PendingConversation{}, access.ErrConflict
	}
	var receipt CoreConversationRegistration
	var err error
	if fresh {
		receipt, err = p.port.BeginConversation(ctx, credential, local.CreationKey)
	} else {
		receipt, err = p.port.LookupConversation(ctx, credential, local.CreationKey)
	}
	if err != nil {
		return local, recoveryError(err)
	}
	if receipt.CreationKey != local.CreationKey || !coreConversationID(receipt.ID) || local.CoreConversationID != "" && local.CoreConversationID != receipt.ID {
		return local, ErrDelegationUnknown
	}
	registered, err := p.repo.BindPending(ctx, owner, local.CreationKey, receipt.ID)
	if err != nil {
		return local, err
	}
	return registered, nil
}

func validPendingConversation(p PendingConversation) bool {
	return p.ID > 0 && p.ID <= maxSafeHistoryID && pendingCreationKey(p.CreationKey) &&
		(p.Phase == "creation_inflight" && p.CoreConversationID == "" || p.Phase == "registered" && coreConversationID(p.CoreConversationID))
}
