package assistant

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"time"
	"unicode/utf8"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

type CoreConversationRegistration struct {
	CreationKey string
	ID          string
}

// DurableDelegationPort is a narrow client port, not proof that a Core issuer
// or provider has been installed. Reserve only reserves the original operation;
// it is deliberately not Model.Next and never authorizes a Go provider relay.
type DurableDelegationPort interface {
	BeginConversation(context.Context, string, string) (CoreConversationRegistration, error)
	LookupConversation(context.Context, string, string) (CoreConversationRegistration, error)
	Issue(context.Context, string, PersonalTextTurnKey) (PersonalTextGrant, error)
	LookupIssue(context.Context, string, PersonalTextTurnKey) (PersonalTextGrant, error)
	Reserve(context.Context, string, PersonalTextReservation) (PersonalTextOperation, error)
	Status(context.Context, string, PersonalTextBinding) (PersonalTextOperation, error)
}

type DelegatedTurns struct {
	authority access.Authority
	journal   DelegationJournal
	port      DurableDelegationPort
	client    *DelegationClient
}

func NewDelegatedTurns(authority access.Authority, journal DelegationJournal, port DurableDelegationPort) (*DelegatedTurns, error) {
	if access.Missing(authority) || access.Missing(journal) || access.Missing(port) {
		return nil, ErrDelegationUnavailable
	}
	// Only Prepare's typed request/hash validation is reused. A nil legacy
	// bridge cannot accidentally enable the API-key model or generic relay path.
	return &DelegatedTurns{authority: authority, journal: journal, port: port, client: &DelegationClient{now: time.Now}}, nil
}

func (s *DelegatedTurns) owner(ctx context.Context, credential string) (int64, error) {
	p, err := s.authority.Check(ctx, credential, access.Account{}, access.Read)
	if err != nil {
		return 0, err
	}
	// The actual candidate Core port supports personal Sessions only. Team and
	// platform-sponsored chat are unavailable, not charged to a substitute user.
	if p.UserID <= 0 || p.Level < 0 || p.Level > 6 || p.Account != (access.Account{Kind: "personal", ID: p.UserID}) {
		return 0, access.ErrForbidden
	}
	return p.UserID, nil
}

func textDelegationRequest(request ModelRequest) (string, error) {
	if !utf8.ValidString(request.Input) || len(request.Input) == 0 || len(request.Input) > 8192 {
		return "", access.ErrInvalid
	}
	if len(request.Tools) != 0 || len(request.Results) != 0 {
		return "", ErrDelegationUnavailable
	}
	encoded, err := json.Marshal(request)
	if err != nil || len(encoded) > 48<<10 {
		return "", access.ErrLimit
	}
	sum := sha256.Sum256(encoded)
	return hex.EncodeToString(sum[:]), nil
}

// OpenTurn verifies the actual local Go conversation owner, then persists the
// original creation and issue claims before calling Core. A retry of an existing
// claim uses ONLY the corresponding read-only lookup; it never reissues a grant.
func (s *DelegatedTurns) OpenTurn(ctx context.Context, credential string, conversationID int64, turnID string, request ModelRequest) (PersonalTextGrant, error) {
	inputHash, err := textDelegationRequest(request)
	if err != nil {
		return PersonalTextGrant{}, err
	}
	owner, err := s.owner(ctx, credential)
	if err != nil {
		return PersonalTextGrant{}, err
	}
	return s.open(ctx, credential, owner, conversationID, turnID, inputHash, true)
}

// RecoverTurn does not create a local conversation, Core owner handle or grant.
// Missing Core receipts remain unknown; absence never authorizes a new Issue.
func (s *DelegatedTurns) RecoverTurn(ctx context.Context, credential string, conversationID int64, turnID string) (PersonalTextGrant, error) {
	owner, err := s.owner(ctx, credential)
	if err != nil {
		return PersonalTextGrant{}, err
	}
	return s.open(ctx, credential, owner, conversationID, turnID, "", false)
}

func (s *DelegatedTurns) open(ctx context.Context, credential string, owner, conversationID int64, turnID, inputHash string, create bool) (PersonalTextGrant, error) {
	var mapping DelegationConversationMapping
	begin := false
	err := s.journal.Within(ctx, owner, conversationID, turnID, func(state *DelegationLocalState) error {
		if state.Turn == nil {
			if !create || !state.CanStart {
				return access.ErrNotFound
			}
			state.Turn = &DelegationStoredTurn{ID: turnID, InputHash: inputHash, Phase: "created", Rounds: []DelegationStoredRound{}}
		} else if create && state.Turn.InputHash != inputHash {
			return access.ErrConflict
		}
		if state.Mapping == nil {
			if !create || !state.CanStart {
				return access.ErrNotFound
			}
			key := freshID()
			if key == "" {
				return access.ErrUnavailable
			}
			state.Mapping = &DelegationConversationMapping{CreationKey: key, Phase: "creation_inflight"}
			begin = true
		}
		mapping = *state.Mapping
		return nil
	})
	if err != nil {
		return PersonalTextGrant{}, err
	}
	if mapping.Phase != "registered" {
		var registration CoreConversationRegistration
		if begin {
			registration, err = s.port.BeginConversation(ctx, credential, mapping.CreationKey)
		} else {
			registration, err = s.port.LookupConversation(ctx, credential, mapping.CreationKey)
		}
		if err != nil {
			return PersonalTextGrant{}, recoveryError(err)
		}
		if registration.CreationKey != mapping.CreationKey || !coreConversationID(registration.ID) {
			return PersonalTextGrant{}, ErrDelegationUnknown
		}
		err = s.journal.Within(ctx, owner, conversationID, turnID, func(state *DelegationLocalState) error {
			if state.Mapping == nil || state.Mapping.CreationKey != mapping.CreationKey || state.Mapping.CoreConversationID != "" && state.Mapping.CoreConversationID != registration.ID {
				return access.ErrConflict
			}
			state.Mapping.CoreConversationID, state.Mapping.Phase = registration.ID, "registered"
			mapping = *state.Mapping
			return nil
		})
		if err != nil {
			return PersonalTextGrant{}, err
		}
	}
	issue := false
	var prior *PersonalTextGrant
	err = s.journal.Within(ctx, owner, conversationID, turnID, func(state *DelegationLocalState) error {
		if state.Mapping == nil || state.Mapping.CoreConversationID != mapping.CoreConversationID || state.Turn == nil {
			return access.ErrConflict
		}
		if state.Turn.Grant != nil {
			copy := *state.Turn.Grant
			prior = &copy
			return nil
		}
		if state.Turn.Phase == "created" && create && state.CanStart {
			state.Turn.Phase = "issue_inflight"
			issue = true
		}
		return nil
	})
	if err != nil {
		return PersonalTextGrant{}, err
	}
	if prior != nil {
		if !validPersonalTextGrant(*prior) || prior.CoreConversationID != mapping.CoreConversationID || prior.TurnID != turnID {
			return PersonalTextGrant{}, access.ErrUnavailable
		}
		return *prior, nil
	}
	input := PersonalTextTurnKey{CoreConversationID: mapping.CoreConversationID, TurnID: turnID}
	var grant PersonalTextGrant
	if issue {
		grant, err = s.port.Issue(ctx, credential, input)
	} else {
		grant, err = s.port.LookupIssue(ctx, credential, input)
	}
	if err != nil {
		return PersonalTextGrant{}, recoveryError(err)
	}
	if !validPersonalTextGrant(grant) || grant.CoreConversationID != mapping.CoreConversationID || grant.TurnID != turnID {
		return PersonalTextGrant{}, ErrDelegationUnknown
	}
	err = s.journal.Within(ctx, owner, conversationID, turnID, func(state *DelegationLocalState) error {
		if state.Turn == nil || state.Mapping == nil || state.Mapping.CoreConversationID != grant.CoreConversationID || state.Turn.Grant != nil && *state.Turn.Grant != grant {
			return access.ErrConflict
		}
		state.Turn.Grant, state.Turn.Phase = &grant, "issued"
		return nil
	})
	return grant, err
}

func recoveryError(err error) error {
	if errors.Is(err, access.ErrNotFound) {
		return ErrDelegationUnknown
	}
	return delegationBridgeError(err)
}

// ReserveRound persists exact payload/op/hash plus an exclusive claim before the
// network call. Existing original rounds are only inspected, never reserved or
// dispatched again. A later round requires its predecessor's settled receipt.
func (s *DelegatedTurns) ReserveRound(ctx context.Context, credential string, conversationID int64, turnID string, round int, request ModelRequest) (PersonalTextOperation, error) {
	inputHash, err := textDelegationRequest(request)
	if err != nil {
		return PersonalTextOperation{}, err
	}
	owner, err := s.owner(ctx, credential)
	if err != nil {
		return PersonalTextOperation{}, err
	}
	var invocation DelegationInvocation
	var reservation PersonalTextReservation
	reserve := false
	err = s.journal.Within(ctx, owner, conversationID, turnID, func(state *DelegationLocalState) error {
		if state.Turn == nil || state.Turn.Grant == nil || state.Mapping == nil || state.Mapping.CoreConversationID != state.Turn.Grant.CoreConversationID || round < 0 || round > len(state.Turn.Rounds) {
			return access.ErrConflict
		}
		if round == 0 && state.Turn.InputHash != inputHash {
			return access.ErrConflict
		}
		if round < len(state.Turn.Rounds) {
			invocation = state.Turn.Rounds[round].Invocation
			reservation = state.Turn.Rounds[round].Reservation
			encoded, err := json.Marshal(request)
			if err != nil || invocation.RequestJSON != string(encoded) {
				return access.ErrConflict
			}
			return nil
		}
		if !state.CanStart || round > 0 && (state.Turn.Rounds[round-1].Outcome == nil || state.Turn.Rounds[round-1].Outcome.Status != "settled") {
			return access.ErrConflict
		}
		var err error
		grant := state.Turn.Grant
		if grant.Revoked {
			return access.ErrForbidden
		}
		invocation, err = s.client.Prepare(DelegationGrant{Version: 1, ID: grant.ID, ConversationID: grant.CoreConversationID, TurnID: grant.TurnID, ExpiresAt: grant.ExpiresAt, MaxModelCalls: grant.MaxModelCalls}, round, request)
		if err != nil {
			return err
		}
		textSum := sha256.Sum256([]byte(request.Input))
		reservation = PersonalTextReservation{Binding: PersonalTextBinding{ID: invocation.ID, CoreConversationID: invocation.ConversationID, TurnID: invocation.TurnID, Round: round, OperationKey: invocation.OperationKey, TextSHA256: hex.EncodeToString(textSum[:])}, Text: request.Input}
		state.Turn.Rounds = append(state.Turn.Rounds, DelegationStoredRound{Invocation: invocation, Reservation: reservation, Phase: "reserve_inflight"})
		reserve = true
		return nil
	})
	if err != nil {
		return PersonalTextOperation{}, err
	}
	var response PersonalTextOperation
	if reserve {
		response, err = s.port.Reserve(ctx, credential, reservation)
	} else {
		response, err = s.port.Status(ctx, credential, reservation.Binding)
	}
	return s.recordOutcome(ctx, owner, conversationID, turnID, reservation.Binding, response, err)
}

func (s *DelegatedTurns) RecoverRound(ctx context.Context, credential string, conversationID int64, turnID string, round int) (PersonalTextOperation, error) {
	owner, err := s.owner(ctx, credential)
	if err != nil {
		return PersonalTextOperation{}, err
	}
	var binding PersonalTextBinding
	err = s.journal.Within(ctx, owner, conversationID, turnID, func(state *DelegationLocalState) error {
		if state.Turn == nil || round < 0 || round >= len(state.Turn.Rounds) {
			return access.ErrNotFound
		}
		binding = state.Turn.Rounds[round].Reservation.Binding
		if !validPersonalTextBinding(binding) {
			return access.ErrUnavailable
		}
		return nil
	})
	if err != nil {
		return PersonalTextOperation{}, err
	}
	response, err := s.port.Status(ctx, credential, binding)
	return s.recordOutcome(ctx, owner, conversationID, turnID, binding, response, err)
}

func (s *DelegatedTurns) recordOutcome(ctx context.Context, owner, conversationID int64, turnID string, binding PersonalTextBinding, response PersonalTextOperation, portError error) (PersonalTextOperation, error) {
	checked, err := checkedPersonalTextOperation(binding, response, portError)
	if err != nil {
		return PersonalTextOperation{}, err
	}
	err = s.journal.Within(ctx, owner, conversationID, turnID, func(state *DelegationLocalState) error {
		if state.Turn == nil || binding.Round < 0 || binding.Round >= len(state.Turn.Rounds) {
			return access.ErrConflict
		}
		r := &state.Turn.Rounds[binding.Round]
		if r.Reservation.Binding != binding {
			return access.ErrConflict
		}
		if r.Outcome != nil && !validPersonalTextTransition(*r.Outcome, checked) {
			return access.ErrConflict
		}
		r.Outcome, r.Phase = &checked, checked.Status
		return nil
	})
	return checked, err
}
