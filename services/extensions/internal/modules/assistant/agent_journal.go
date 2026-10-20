package assistant

import (
	"bytes"
	"context"
	"encoding/json"
	"unicode/utf8"

	"github.com/TokenNotIncluded/api.lmm.best/extensions/internal/modules/assistant/access"
)

// Within owns a separate Go business transaction. It verifies and locks the
// actual owned conversation, and performs no network calls in its callback.
type AgentJournal interface {
	WithinAgent(context.Context, int64, int64, string, func(*AgentLocalState) error) error
}
type AgentLocalState struct {
	CanStart bool
	Mapping  *DelegationConversationMapping
	Turn     *AgentStoredTurn
}
type AgentStoredTurn struct {
	ID          string                      `json:"id"`
	Input       string                      `json:"input"`
	InputSHA256 string                      `json:"input_sha256"`
	Phase       string                      `json:"phase"`
	Snapshot    *AgentTurnSnapshot          `json:"snapshot,omitempty"`
	Tools       map[string]*AgentStoredTool `json:"tools"`
}
type AgentStoredTool struct {
	Call           AgentToolCall      `json:"call"`
	ClaimKey       string             `json:"claim_key"`
	Phase          string             `json:"phase"`
	Result         *AgentToolResult   `json:"result,omitempty"`
	Receipt        *AgentToolReceipt  `json:"receipt,omitempty"`
	ReceiptHistory []AgentToolReceipt `json:"receipt_history,omitempty"`
}

func sameAgentJSON(a, b any) bool {
	left, e1 := json.Marshal(a)
	right, e2 := json.Marshal(b)
	return e1 == nil && e2 == nil && bytes.Equal(left, right)
}
func terminalAgentPhase(phase AgentTurnPhase) bool {
	return phase == AgentTurnCompleted || phase == AgentTurnFailed || phase == AgentTurnExpired
}
func terminalAgentResult(state AgentToolState) bool {
	return state == AgentToolSucceeded || state == AgentToolFailed
}

func validAgentSnapshot(snapshot AgentTurnSnapshot, key AgentTurnKey, owner int64, inputHash string) bool {
	if snapshot.Key != key || !validAgentKey(key) || snapshot.OwnerUserID != owner || owner <= 0 || snapshot.CoreOwnerAccountID <= 0 || snapshot.Account.Kind != "personal" || snapshot.Account.ID != owner || snapshot.OriginalTextSHA256 != inputHash || !lowerDigest(inputHash) || snapshot.Revision == 0 || snapshot.MaxRounds < 1 || snapshot.MaxRounds > 32 || snapshot.Round >= snapshot.MaxRounds || snapshot.BudgetCredits <= 0 || snapshot.BudgetCredits > maxSafeHistoryID || snapshot.HeldCredits < 0 || snapshot.SettledCredits < 0 || snapshot.SettledCredits > snapshot.BudgetCredits || snapshot.HeldCredits > snapshot.BudgetCredits-snapshot.SettledCredits || len(snapshot.PendingTools) > 8 {
		return false
	}
	switch snapshot.Phase {
	case AgentTurnNew, AgentTurnRunning, AgentTurnAwaitingTools, AgentTurnCompleted, AgentTurnFailed, AgentTurnUnknown, AgentTurnExpired:
	default:
		return false
	}
	if snapshot.Phase == AgentTurnCompleted {
		if !utf8.ValidString(snapshot.FinalText) || len(snapshot.FinalText) == 0 || len(snapshot.FinalText) > 32768 || len(snapshot.PendingTools) != 0 {
			return false
		}
	} else if snapshot.FinalText != "" {
		return false
	}
	if snapshot.Phase != AgentTurnAwaitingTools && len(snapshot.PendingTools) != 0 {
		return false
	}
	seen := map[string]bool{}
	for _, call := range snapshot.PendingTools {
		if !validAgentCall(call) || call.CoreConversationID != key.CoreConversationID || call.TurnID != key.ClientTurnID || call.Round != snapshot.Round || seen[call.OperationID] {
			return false
		}
		seen[call.OperationID] = true
	}
	return snapshot.Phase != AgentTurnAwaitingTools || len(snapshot.PendingTools) > 0
}

func validAgentSnapshotTransition(before, after AgentTurnSnapshot) bool {
	if before.Key != after.Key || before.OwnerUserID != after.OwnerUserID || before.CoreOwnerAccountID != after.CoreOwnerAccountID || before.Account != after.Account || before.OriginalTextSHA256 != after.OriginalTextSHA256 || before.MaxRounds != after.MaxRounds || before.BudgetCredits != after.BudgetCredits || after.Revision < before.Revision || after.Round < before.Round || after.SettledCredits < before.SettledCredits {
		return false
	}
	if after.Revision == before.Revision || terminalAgentPhase(before.Phase) {
		return sameAgentJSON(before, after)
	}
	return true
}

func validAgentStoredTurn(turn *AgentStoredTurn) bool {
	if turn == nil || !access.ID(turn.ID) || len(turn.ID) > 64 || len(turn.Input) == 0 || len(turn.Input) > 8192 || !utf8.ValidString(turn.Input) || agentDigest([]byte(turn.Input)) != turn.InputSHA256 || turn.Tools == nil || len(turn.Tools) > 256 {
		return false
	}
	switch turn.Phase {
	case "created", "start_inflight":
		if turn.Snapshot != nil || len(turn.Tools) != 0 {
			return false
		}
	case "started":
		if turn.Snapshot == nil || turn.Snapshot.Key.ClientTurnID != turn.ID || !validAgentSnapshot(*turn.Snapshot, turn.Snapshot.Key, turn.Snapshot.OwnerUserID, turn.InputSHA256) {
			return false
		}
	default:
		return false
	}
	for id, tool := range turn.Tools {
		if tool == nil || tool.Call.OperationID != id || tool.Call.TurnID != turn.ID || !validAgentCall(tool.Call) || !pendingCreationKey(tool.ClaimKey) {
			return false
		}
		switch tool.Phase {
		case "claim_inflight", "inspect_only", "executing", "result_stored", "core_accepted", "unknown":
		default:
			return false
		}
		if tool.Result != nil {
			if _, err := checkedAgentResult(tool.Call, *tool.Result, ""); err != nil {
				return false
			}
		}
		if tool.Receipt != nil && (tool.Result == nil || !validAgentToolReceipt(tool.Call, tool.ClaimKey, *tool.Result, *tool.Receipt)) {
			return false
		}
		if tool.Phase == "core_accepted" && tool.Receipt == nil {
			return false
		}
		if len(tool.ReceiptHistory) > 1 {
			return false
		}
		for _, receipt := range tool.ReceiptHistory {
			if receipt.OperationID != id || receipt.ClaimKey != tool.ClaimKey || receipt.State != AgentToolUnknown || !lowerDigest(receipt.ResultSHA256) {
				return false
			}
		}
	}
	return true
}

func validAgentStoredUpdate(before, after *AgentStoredTurn) bool {
	if !validAgentStoredTurn(after) {
		return false
	}
	if before == nil {
		return true
	}
	if before.ID != after.ID || before.Input != after.Input || before.InputSHA256 != after.InputSHA256 || before.Phase == "start_inflight" && after.Phase == "created" || before.Phase == "started" && after.Phase != "started" || len(after.Tools) < len(before.Tools) {
		return false
	}
	if before.Snapshot != nil && (after.Snapshot == nil || !validAgentSnapshotTransition(*before.Snapshot, *after.Snapshot)) {
		return false
	}
	for id, old := range before.Tools {
		next := after.Tools[id]
		if next == nil || !sameAgentJSON(old.Call, next.Call) || old.ClaimKey != next.ClaimKey || old.Result != nil && terminalAgentResult(old.Result.State) && !sameAgentJSON(old.Result, next.Result) {
			return false
		}
		if old.Receipt != nil && !sameAgentJSON(old.Receipt, next.Receipt) {
			if old.Receipt.State != AgentToolUnknown || next.Result == nil || !terminalAgentResult(next.Result.State) || next.Receipt != nil && next.Receipt.State != AgentToolSucceeded || len(next.ReceiptHistory) != len(old.ReceiptHistory)+1 || !sameAgentJSON(next.ReceiptHistory[len(next.ReceiptHistory)-1], *old.Receipt) {
				return false
			}
		} else if len(next.ReceiptHistory) != len(old.ReceiptHistory) {
			return false
		}
		if len(next.ReceiptHistory) < len(old.ReceiptHistory) {
			return false
		}
		for i := range old.ReceiptHistory {
			if !sameAgentJSON(next.ReceiptHistory[i], old.ReceiptHistory[i]) {
				return false
			}
		}
	}
	return true
}

func validAgentToolReceipt(call AgentToolCall, claim string, result AgentToolResult, receipt AgentToolReceipt) bool {
	if receipt.OperationID != call.OperationID || receipt.ClaimKey != claim || receipt.ResultSHA256 != result.ResultSHA256 || receipt.DomainReceiptID != result.DomainReceiptID {
		return false
	}
	if terminalAgentResult(result.State) {
		return receipt.State == AgentToolSucceeded
	}
	return result.State == AgentToolUnknown && receipt.State == AgentToolUnknown
}
