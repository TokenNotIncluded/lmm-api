package assistant

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/TokenNotIncluded/lmm-api/services/extensions/internal/modules/assistant/access"
)

// AgentChat consumes Core-owned model rounds. Its journal commits an intent
// before each outbound start/claim, so an uncertain acknowledgement can only
// lead to inspection of the original operation, never another execution.
type AgentChat struct {
	authority    access.Authority
	pending      *PendingConversations
	journal      AgentJournal
	port         AgentTurnPort
	registry     *AgentRegistry
	history      *HistoryService
	gifts        *Gifts
	humanSupport HumanSupportGuard
}

type HumanSupportGuard interface {
	HumanSupportActive(context.Context, string, string) (bool, error)
}

func (s *AgentChat) MultiRoundAvailable() bool { return s != nil }

func (s *AgentChat) WithHumanSupportGuard(guard HumanSupportGuard) *AgentChat {
	copy := *s
	copy.humanSupport = guard
	return &copy
}
func (s *AgentChat) paused(ctx context.Context, credential, coreID string) (bool, error) {
	if access.Missing(s.humanSupport) {
		return false, nil
	}
	return s.humanSupport.HumanSupportActive(ctx, credential, coreID)
}

func NewAgentChat(authority access.Authority, pending *PendingConversations, journal AgentJournal, port AgentTurnPort, registry *AgentRegistry, history *HistoryService) (*AgentChat, error) {
	if access.Missing(authority) || pending == nil || access.Missing(journal) || access.Missing(port) || registry == nil || history == nil {
		return nil, ErrDelegationUnavailable
	}
	return &AgentChat{authority: authority, pending: pending, journal: journal, port: port, registry: registry, history: history}, nil
}

// WithGifts composes the independently negotiated stored-gift port without
// changing the agent turn's payer or creating a gift evaluation authority.
func (s *AgentChat) WithGifts(gifts *Gifts) *AgentChat { copy := *s; copy.gifts = gifts; return &copy }
func (s *AgentChat) GiftsAvailable() bool              { return s != nil && s.gifts != nil }
func (s *AgentChat) GetNewUserGift(ctx context.Context, credential string) (*NewUserGift, error) {
	if !s.GiftsAvailable() {
		return nil, ErrDelegationUnavailable
	}
	return s.gifts.GetNewUserGift(ctx, credential)
}
func (s *AgentChat) ClaimStoredNewUserGift(ctx context.Context, credential string) (GiftClaim, error) {
	if !s.GiftsAvailable() {
		return GiftClaim{}, ErrDelegationUnavailable
	}
	return s.gifts.ClaimStoredNewUserGift(ctx, credential)
}

func (s *AgentChat) actor(ctx context.Context, credential string) (access.Principal, error) {
	p, err := s.authority.Check(ctx, credential, access.Account{}, access.Read)
	if err != nil {
		return p, err
	}
	if p.UserID <= 0 || p.Level < 0 || p.Level > 6 || p.Account != (access.Account{Kind: "personal", ID: p.UserID}) {
		return p, access.ErrForbidden
	}
	return p, nil
}

func validAgentChatID(conversation int64, turn string) bool {
	return conversation > 0 && conversation <= maxSafeHistoryID && access.ID(turn) && len(turn) <= 64
}

func (s *AgentChat) Chat(ctx context.Context, credential string, request PersonalChatRequest) (PersonalChatReply, error) {
	if request.ConversationID < 0 || request.ConversationID > maxSafeHistoryID || !access.ID(request.ClientTurnID) || len(request.ClientTurnID) > 64 || !utf8.ValidString(request.Message) || len(request.Message) > 8192 || strings.TrimSpace(request.Message) == "" {
		return PersonalChatReply{}, access.ErrInvalid
	}
	p, err := s.actor(ctx, credential)
	if err != nil {
		return PersonalChatReply{}, err
	}
	id := request.ConversationID
	if id == 0 {
		sum := sha256.Sum256([]byte("personal-chat-v1:" + request.ClientTurnID))
		pending, e := s.pending.Start(ctx, credential, hex.EncodeToString(sum[:16]))
		if e != nil {
			return PersonalChatReply{}, e
		}
		id = pending.ID
	}
	var key AgentTurnKey
	if !access.Missing(s.humanSupport) {
		// Read the owned mapping before creating a model intent. An active human
		// conversation must not leave an unsent start intent that a retry mistakes
		// for a lost Core acknowledgement.
		err = s.journal.WithinAgent(ctx, p.UserID, id, request.ClientTurnID, func(local *AgentLocalState) error {
			if local.Mapping == nil || local.Mapping.Phase != "registered" {
				return ErrDelegationUnavailable
			}
			if local.Turn != nil && (local.Turn.Input != request.Message || local.Turn.InputSHA256 != agentDigest([]byte(request.Message))) {
				return access.ErrConflict
			}
			key = AgentTurnKey{local.Mapping.CoreConversationID, request.ClientTurnID}
			return nil
		})
		if err != nil {
			return PersonalChatReply{}, err
		}
		if paused, pauseErr := s.paused(ctx, credential, key.CoreConversationID); pauseErr != nil {
			return PersonalChatReply{}, pauseErr
		} else if paused {
			return PersonalChatReply{ConversationID: id, ClientTurnID: request.ClientTurnID, State: "human_support_pending"}, nil
		}
	}
	fresh := false
	err = s.journal.WithinAgent(ctx, p.UserID, id, request.ClientTurnID, func(local *AgentLocalState) error {
		if local.Mapping == nil || local.Mapping.Phase != "registered" || !coreConversationID(local.Mapping.CoreConversationID) {
			return ErrDelegationUnavailable
		}
		key = AgentTurnKey{local.Mapping.CoreConversationID, request.ClientTurnID}
		if local.Turn != nil {
			if local.Turn.Input != request.Message || local.Turn.InputSHA256 != agentDigest([]byte(request.Message)) {
				return access.ErrConflict
			}
			return nil
		}
		if !local.CanStart {
			return access.ErrForbidden
		}
		local.Turn = &AgentStoredTurn{ID: request.ClientTurnID, Input: request.Message, InputSHA256: agentDigest([]byte(request.Message)), Phase: "start_inflight", Tools: map[string]*AgentStoredTool{}}
		fresh = true
		return nil
	})
	if err != nil {
		return PersonalChatReply{}, err
	}
	var snap AgentTurnSnapshot
	if fresh {
		snap, err = s.port.StartAgentTurn(ctx, credential, AgentTurnStart{Key: key, Text: request.Message})
	} else {
		snap, err = s.port.GetAgentTurn(ctx, credential, key)
	}
	if err != nil {
		return PersonalChatReply{ConversationID: id, ClientTurnID: request.ClientTurnID, State: string(AgentTurnUnknown)}, agentRecoveryError(err)
	}
	return s.advance(ctx, credential, p, id, request.ClientTurnID, snap, false)
}

// Recover never starts a model turn or executes an extension effect. Completed
// local results may be acknowledged to Core under their original claim key.
func (s *AgentChat) Recover(ctx context.Context, credential string, conversation int64, turn string) (PersonalChatReply, error) {
	if !validAgentChatID(conversation, turn) {
		return PersonalChatReply{}, access.ErrInvalid
	}
	p, err := s.actor(ctx, credential)
	if err != nil {
		return PersonalChatReply{}, err
	}
	var key AgentTurnKey
	err = s.journal.WithinAgent(ctx, p.UserID, conversation, turn, func(local *AgentLocalState) error {
		if local.Turn == nil {
			return access.ErrNotFound
		}
		if local.Mapping == nil || local.Mapping.Phase != "registered" || !coreConversationID(local.Mapping.CoreConversationID) {
			return ErrDelegationUnavailable
		}
		key = AgentTurnKey{local.Mapping.CoreConversationID, turn}
		return nil
	})
	if err != nil {
		return PersonalChatReply{}, err
	}
	snap, err := s.port.GetAgentTurn(ctx, credential, key)
	if err != nil {
		return PersonalChatReply{}, agentRecoveryError(err)
	}
	return s.advance(ctx, credential, p, conversation, turn, snap, true)
}

// Resume is an explicit continuation command for the original durable user
// turn. It accepts no new text or authority and never reexecutes an old claim.
func (s *AgentChat) Resume(ctx context.Context, credential string, conversation int64, turn string) (PersonalChatReply, error) {
	if !validAgentChatID(conversation, turn) {
		return PersonalChatReply{}, access.ErrInvalid
	}
	p, err := s.actor(ctx, credential)
	if err != nil {
		return PersonalChatReply{}, err
	}
	var input string
	err = s.journal.WithinAgent(ctx, p.UserID, conversation, turn, func(local *AgentLocalState) error {
		if local.Turn == nil {
			return access.ErrNotFound
		}
		input = local.Turn.Input
		return nil
	})
	if err != nil {
		return PersonalChatReply{}, err
	}
	return s.Chat(ctx, credential, PersonalChatRequest{ConversationID: conversation, ClientTurnID: turn, Message: input})
}

func (s *AgentChat) storeSnapshot(ctx context.Context, p access.Principal, conversation int64, turn string, snap AgentTurnSnapshot) (AgentStoredTurn, error) {
	var stored AgentStoredTurn
	err := s.journal.WithinAgent(ctx, p.UserID, conversation, turn, func(local *AgentLocalState) error {
		if local.Turn == nil || local.Mapping == nil {
			return access.ErrNotFound
		}
		key := AgentTurnKey{local.Mapping.CoreConversationID, turn}
		if !validAgentSnapshot(snap, key, p.UserID, local.Turn.InputSHA256) {
			return ErrDelegationUnknown
		}
		if before := local.Turn.Snapshot; before != nil {
			// A concurrent inspector can return an older valid receipt. It cannot
			// roll local state back or replace the immutable original binding.
			if snap.Revision < before.Revision {
				if !validAgentSnapshotTransition(snap, *before) {
					return access.ErrConflict
				}
				stored = *local.Turn
				return nil
			}
			if !validAgentSnapshotTransition(*before, snap) {
				return access.ErrConflict
			}
		}
		local.Turn.Phase = "started"
		local.Turn.Snapshot = &snap
		stored = *local.Turn
		return nil
	})
	return stored, err
}

func (s *AgentChat) advance(ctx context.Context, credential string, p access.Principal, conversation int64, turn string, snap AgentTurnSnapshot, recoverOnly bool) (PersonalChatReply, error) {
	reply := PersonalChatReply{ConversationID: conversation, ClientTurnID: turn}
	// One request is bounded even if Core or a tool keeps returning pending.
	deadline := time.Now().Add(15 * time.Second)
	for step := 0; step < 512; step++ {
		if paused, pauseErr := s.paused(ctx, credential, snap.Key.CoreConversationID); pauseErr != nil {
			return reply, pauseErr
		} else if paused {
			reply.State = "human_support_pending"
			return reply, nil
		}
		stored, err := s.storeSnapshot(ctx, p, conversation, turn, snap)
		if err != nil {
			return reply, err
		}
		snap = *stored.Snapshot
		reply.State = string(snap.Phase)
		if snap.Phase == AgentTurnCompleted {
			id, err := s.history.RecordCompletedAgentTurn(ctx, credential, conversation, turn, stored.Input, snap.FinalText, agentCompletedTitle(stored))
			if err != nil {
				return reply, err
			}
			if id != conversation {
				return reply, access.ErrConflict
			}
			reply.State = "settled"
			reply.Text = RedactAssistantHistoryContent(snap.FinalText)
			return reply, nil
		}
		if !recoverOnly && (snap.Phase == AgentTurnNew || snap.Phase == AgentTurnRunning) && time.Now().Before(deadline) {
			timer := time.NewTimer(50 * time.Millisecond)
			select {
			case <-ctx.Done():
				timer.Stop()
				return reply, ErrDelegationUnknown
			case <-timer.C:
			}
			next, getErr := s.port.GetAgentTurn(ctx, credential, snap.Key)
			if getErr != nil {
				return reply, agentRecoveryError(getErr)
			}
			snap = next
			continue
		}
		if snap.Phase != AgentTurnAwaitingTools {
			return reply, nil
		}
		progressed := false
		for _, call := range snap.PendingTools {
			done, err := s.consumeTool(ctx, credential, p, conversation, turn, stored.Input, call, recoverOnly)
			if err != nil {
				return reply, err
			}
			progressed = progressed || done
		}
		if !progressed {
			return reply, nil
		}
		next, err := s.port.GetAgentTurn(ctx, credential, snap.Key)
		if err != nil {
			return reply, agentRecoveryError(err)
		}
		for sameAgentJSON(snap, next) && !recoverOnly && time.Now().Before(deadline) {
			timer := time.NewTimer(50 * time.Millisecond)
			select {
			case <-ctx.Done():
				timer.Stop()
				return reply, ErrDelegationUnknown
			case <-timer.C:
			}
			next, err = s.port.GetAgentTurn(ctx, credential, snap.Key)
			if err != nil {
				return reply, agentRecoveryError(err)
			}
		}
		if sameAgentJSON(snap, next) {
			return reply, nil
		}
		snap = next
	}
	return reply, access.ErrLimit
}

func (s *AgentChat) consumeTool(ctx context.Context, credential string, p access.Principal, conversation int64, turn, input string, call AgentToolCall, recoverOnly bool) (bool, error) {
	var tool AgentStoredTool
	fresh := false
	unclaimed := false
	err := s.journal.WithinAgent(ctx, p.UserID, conversation, turn, func(local *AgentLocalState) error {
		if local.Turn == nil || local.Turn.Snapshot == nil {
			return access.ErrNotFound
		}
		current, exists := local.Turn.Tools[call.OperationID]
		if exists {
			if !sameAgentJSON(current.Call, call) {
				return access.ErrConflict
			}
		} else {
			// Recovery cannot create a new local/Core claim. A later normal Chat
			// may claim this never-dispatched operation under the same turn.
			if recoverOnly {
				unclaimed = true
				return nil
			}
			current = &AgentStoredTool{Call: call, ClaimKey: freshID(), Phase: "claim_inflight"}
			if !pendingCreationKey(current.ClaimKey) {
				return access.ErrUnavailable
			}
			local.Turn.Tools[call.OperationID] = current
			fresh = !recoverOnly
		}
		tool = *current
		return nil
	})
	if err != nil {
		return false, err
	}
	if unclaimed {
		return false, nil
	}
	if tool.Receipt != nil && tool.Receipt.State != AgentToolUnknown {
		return false, nil
	}
	claimInput := AgentToolClaimInput{Key: AgentTurnKey{call.CoreConversationID, call.TurnID}, OperationID: call.OperationID, ClaimKey: tool.ClaimKey}
	if tool.Receipt != nil {
		fresh = false
	}
	if tool.Result == nil || !terminalAgentResult(tool.Result.State) {
		claim, err := s.port.ClaimAgentTool(ctx, credential, claimInput)
		if err != nil {
			return false, agentRecoveryError(err)
		}
		if claim.ClaimKey != tool.ClaimKey || !sameAgentJSON(claim.Call, call) {
			return false, ErrDelegationUnknown
		}
		if claim.AcceptedResult != nil {
			// A receipt without our original persisted payload cannot reconstruct a
			// business result. Inspect the original domain operation instead.
			fresh = false
		}
		execute := fresh && claim.MayExecute
		err = s.journal.WithinAgent(ctx, p.UserID, conversation, turn, func(local *AgentLocalState) error {
			t := local.Turn.Tools[call.OperationID]
			if t == nil || !sameAgentJSON(t.Call, call) || t.ClaimKey != tool.ClaimKey {
				return access.ErrConflict
			}
			if t.Result != nil && terminalAgentResult(t.Result.State) {
				tool = *t
				execute = false
				return nil
			}
			if execute {
				t.Phase = "executing"
			} else {
				t.Phase = "inspect_only"
			}
			return nil
		})
		if err != nil {
			return false, err
		}
		if tool.Result == nil || !terminalAgentResult(tool.Result.State) {
			// Recheck the Session immediately before touching the domain adapter.
			actor, err := s.actor(ctx, credential)
			if err != nil {
				return false, err
			}
			if actor.UserID != p.UserID || actor.Account != p.Account {
				return false, access.ErrForbidden
			}
			invocation := AgentInvocation{Credential: credential, Actor: actor, Call: call, OriginalUserText: input}
			var result AgentToolResult
			if execute {
				result, err = s.registry.Execute(ctx, invocation)
			} else {
				result, err = s.registry.Inspect(ctx, invocation)
			}
			if err != nil {
				return false, agentRecoveryError(err)
			}
			// An acknowledged UNKNOWN observation remains immutable until an
			// inspection proves the terminal original outcome. Repeated pending
			// observations neither replace its digest nor resubmit completion.
			if tool.Receipt != nil && tool.Receipt.State == AgentToolUnknown && !terminalAgentResult(result.State) {
				return false, nil
			}
			err = s.journal.WithinAgent(ctx, p.UserID, conversation, turn, func(local *AgentLocalState) error {
				t := local.Turn.Tools[call.OperationID]
				if t == nil || !sameAgentJSON(t.Call, call) || t.ClaimKey != tool.ClaimKey {
					return access.ErrConflict
				}
				if t.Result != nil && terminalAgentResult(t.Result.State) {
					tool = *t
					return nil
				}
				if t.Receipt != nil && t.Receipt.State == AgentToolUnknown && terminalAgentResult(result.State) {
					t.ReceiptHistory = append(t.ReceiptHistory, *t.Receipt)
					t.Receipt = nil
				}
				t.Result = &result
				t.Phase = "result_stored"
				tool = *t
				return nil
			})
			if err != nil {
				return false, err
			}
		}
	}
	if tool.Result == nil || tool.Result.State == AgentToolPending {
		return false, nil
	}
	receipt, err := s.port.CompleteAgentTool(ctx, credential, AgentToolCompletion{Claim: claimInput, Result: *tool.Result})
	if err != nil {
		return false, agentRecoveryError(err)
	}
	if !validAgentToolReceipt(call, tool.ClaimKey, *tool.Result, receipt) {
		return false, ErrDelegationUnknown
	}
	err = s.journal.WithinAgent(ctx, p.UserID, conversation, turn, func(local *AgentLocalState) error {
		t := local.Turn.Tools[call.OperationID]
		if t == nil || !sameAgentJSON(t.Call, call) || t.ClaimKey != tool.ClaimKey || !sameAgentJSON(t.Result, tool.Result) {
			return access.ErrConflict
		}
		if t.Receipt != nil && !sameAgentJSON(t.Receipt, receipt) {
			if t.Receipt.State != AgentToolUnknown || receipt.State != AgentToolSucceeded {
				return access.ErrConflict
			}
			t.ReceiptHistory = append(t.ReceiptHistory, *t.Receipt)
		}
		t.Receipt = &receipt
		t.Phase = "core_accepted"
		return nil
	})
	return err == nil && receipt.State == AgentToolSucceeded, err
}

// Title is optional first-turn metadata. Only one Core-accepted successful
// title result can provide it; ambiguous, stale or malformed proposals fall
// back to the history service's ordinary redacted-input title.
func agentCompletedTitle(turn AgentStoredTurn) string {
	title := ""
	count := 0
	for _, tool := range turn.Tools {
		if tool.Call.Scope.Name != "set_conversation_title" || tool.Result == nil || tool.Result.State != AgentToolSucceeded || tool.Receipt == nil || tool.Phase != "core_accepted" {
			continue
		}
		count++
		var value struct {
			Title string `json:"title"`
		}
		if json.Unmarshal(tool.Result.ResultJSON, &value) != nil {
			return ""
		}
		value.Title = strings.TrimSpace(RedactAssistantHistoryContent(value.Title))
		if !utf8.ValidString(value.Title) || len([]rune(value.Title)) < 2 || len([]rune(value.Title)) > 60 || strings.ContainsRune(value.Title, 0) {
			return ""
		}
		title = value.Title
	}
	if count != 1 {
		return ""
	}
	return title
}
