package assistant

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
)

// DelegationJournal owns private Go business records, never Core authority.
// Within must lock and verify the actual local conversation row against the
// live Core-authenticated user before exposing this state. No network I/O is
// permitted inside the callback. A failed COMMIT has an uncertain outcome.
type DelegationJournal interface {
	Within(context.Context, int64, int64, string, func(*DelegationLocalState) error) error
}

// Original claims and receipts cannot be rewritten while recovering an ACK.
// A state transition never supplies authority to create a replacement operation.
func validStoredDelegationUpdate(before, after *DelegationStoredTurn) bool {
	if !validStoredDelegationTurn(after) {
		return false
	}
	if before == nil {
		return true
	}
	if before.ID != after.ID || before.InputHash != after.InputHash ||
		before.Phase == "issue_inflight" && after.Phase == "created" ||
		before.Grant != nil && (after.Grant == nil || *before.Grant != *after.Grant) ||
		len(after.Rounds) < len(before.Rounds) {
		return false
	}
	for i, old := range before.Rounds {
		next := after.Rounds[i]
		if old.Invocation != next.Invocation || old.Reservation != next.Reservation ||
			old.Outcome != nil && (next.Outcome == nil || !validPersonalTextTransition(*old.Outcome, *next.Outcome)) {
			return false
		}
	}
	return true
}

func validPersonalTextTransition(before, after PersonalTextOperation) bool {
	if before.Binding != after.Binding || before.RequestID != after.RequestID || before.ReservedCredits != after.ReservedCredits {
		return false
	}
	if before.Status == "settled" || before.Status == "released" {
		old, _ := json.Marshal(before)
		next, _ := json.Marshal(after)
		return bytes.Equal(old, next)
	}
	return true
}

type DelegationLocalState struct {
	CanStart bool
	Mapping  *DelegationConversationMapping
	Turn     *DelegationStoredTurn
}

type DelegationConversationMapping struct {
	CreationKey        string `json:"creation_key"`
	CoreConversationID string `json:"core_conversation_id"`
	Phase              string `json:"phase"`
}

type DelegationStoredTurn struct {
	ID        string                  `json:"id"`
	InputHash string                  `json:"input_hash"`
	Phase     string                  `json:"phase"`
	Grant     *PersonalTextGrant      `json:"grant"`
	Rounds    []DelegationStoredRound `json:"rounds"`
}

type DelegationStoredRound struct {
	Invocation  DelegationInvocation    `json:"invocation"`
	Reservation PersonalTextReservation `json:"reservation"`
	Phase       string                  `json:"phase"`
	Outcome     *PersonalTextOperation  `json:"outcome"`
}

func validStoredDelegationTurn(turn *DelegationStoredTurn) bool {
	if turn == nil || !delegationLabel(turn.ID, 124) || !lowerDigest(turn.InputHash) || len(turn.Rounds) > 4 {
		return false
	}
	if turn.Grant == nil {
		return len(turn.Rounds) == 0 && (turn.Phase == "created" || turn.Phase == "issue_inflight")
	}
	g := turn.Grant
	if turn.Phase != "issued" || !validPersonalTextGrant(*g) || g.TurnID != turn.ID || len(turn.Rounds) > g.MaxModelCalls {
		return false
	}
	for i, round := range turn.Rounds {
		r := round.Invocation
		sum := sha256.Sum256([]byte(r.RequestJSON))
		if !validDelegationBinding(r.DelegationBinding) || r.Round != i || r.ID != g.ID || r.ConversationID != g.CoreConversationID || r.TurnID != turn.ID || len(r.RequestJSON) == 0 || len(r.RequestJSON) > 48<<10 || hex.EncodeToString(sum[:]) != r.PayloadSHA256 || i == 0 && turn.InputHash != r.PayloadSHA256 {
			return false
		}
		var request ModelRequest
		if json.Unmarshal([]byte(r.RequestJSON), &request) != nil || !validPersonalTextReservation(round.Reservation) || round.Reservation.Text != request.Input || len(request.Tools) != 0 || len(request.Results) != 0 || round.Reservation.Binding.ID != g.ID || round.Reservation.Binding.CoreConversationID != g.CoreConversationID || round.Reservation.Binding.TurnID != turn.ID || round.Reservation.Binding.Round != i {
			return false
		}
		if i > 0 && (turn.Rounds[i-1].Outcome == nil || turn.Rounds[i-1].Outcome.Status != "settled") {
			return false
		}
		if round.Outcome == nil {
			if round.Phase != "reserve_inflight" {
				return false
			}
		} else if _, err := checkedPersonalTextOperation(round.Reservation.Binding, *round.Outcome, nil); err != nil || round.Phase != round.Outcome.Status {
			return false
		}
	}
	return true
}
