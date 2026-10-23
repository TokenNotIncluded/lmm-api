package bounties

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
)

// LegacyEscrowCommand is a frozen historical claim operation. Its implementations
// contain no caller-selected amount, recipient or source ownership.
type LegacyEscrowCommand interface {
	legacyEscrow() (kind, migrationID string, projectID int64, operationKey string)
}

type LegacyEscrowSubmitCommand struct {
	MigrationID       string `json:"migration_id"`
	SourceProjectID   int64  `json:"source_project_id"`
	SourceChallengeID int64  `json:"source_challenge_id"`
	OperationKey      string `json:"operation_key"`
	IssueURL          string `json:"issue_url"`
	PullRequestURL    string `json:"pull_request_url"`
	SubmissionNote    string `json:"submission_note"`
}

type LegacyEscrowPayoutCommand struct {
	MigrationID            string  `json:"migration_id"`
	SourceProjectID        int64   `json:"source_project_id"`
	SourceChallengeID      int64   `json:"source_challenge_id"`
	OperationKey           string  `json:"operation_key"`
	SubmissionOperationKey *string `json:"submission_operation_key,omitempty"`
}

type LegacyEscrowCloseCommand struct {
	MigrationID     string `json:"migration_id"`
	SourceProjectID int64  `json:"source_project_id"`
	OperationKey    string `json:"operation_key"`
}

func (c LegacyEscrowSubmitCommand) legacyEscrow() (string, string, int64, string) {
	return "submit", c.MigrationID, c.SourceProjectID, c.OperationKey
}
func (c LegacyEscrowPayoutCommand) legacyEscrow() (string, string, int64, string) {
	return "payout", c.MigrationID, c.SourceProjectID, c.OperationKey
}
func (c LegacyEscrowCloseCommand) legacyEscrow() (string, string, int64, string) {
	return "close", c.MigrationID, c.SourceProjectID, c.OperationKey
}

type LegacyEscrowReceipt struct {
	Status         string  `json:"status"`
	Code           *string `json:"code"`
	OperationKey   string  `json:"operation_key"`
	JournalID      *int64  `json:"journal_id"`
	RemainingUnits int64   `json:"remaining_units"`
	Revision       int64   `json:"revision"`
}

type LegacyEscrowObservation struct {
	Command     LegacyEscrowCommand `json:"-"`
	ActorUserID int64               `json:"actor_user_id"`
	Receipt     LegacyEscrowReceipt `json:"receipt"`
}

func (o LegacyEscrowObservation) MarshalJSON() ([]byte, error) {
	command, valid := legacyEscrowTagged(o.Command)
	if !valid {
		return nil, ErrInvalid
	}
	return json.Marshal(struct {
		Command     any                 `json:"command"`
		ActorUserID int64               `json:"actor_user_id"`
		Receipt     LegacyEscrowReceipt `json:"receipt"`
	}{command, o.ActorUserID, o.Receipt})
}

func legacyEscrowTagged(command LegacyEscrowCommand) (any, bool) {
	switch v := command.(type) {
	case LegacyEscrowSubmitCommand:
		return struct {
			Kind string `json:"kind"`
			LegacyEscrowSubmitCommand
		}{"submit", v}, v.SourceChallengeID > 0
	case LegacyEscrowPayoutCommand:
		return struct {
			Kind string `json:"kind"`
			LegacyEscrowPayoutCommand
		}{"payout", v}, v.SourceChallengeID > 0 && (v.SubmissionOperationKey == nil || legacyEscrowKey(*v.SubmissionOperationKey))
	case LegacyEscrowCloseCommand:
		return struct {
			Kind string `json:"kind"`
			LegacyEscrowCloseCommand
		}{"close", v}, true
	default:
		return nil, false
	}
}

func legacyEscrowKey(v string) bool {
	if len(v) == 0 || len(v) > 128 {
		return false
	}
	for _, ch := range []byte(v) {
		if !(ch >= 'a' && ch <= 'z' || ch >= 'A' && ch <= 'Z' || ch >= '0' && ch <= '9' || ch == '.' || ch == '_' || ch == ':' || ch == '/' || ch == '-') {
			return false
		}
	}
	return true
}

func validLegacyEscrowCommand(command LegacyEscrowCommand) bool {
	if command == nil {
		return false
	}
	_, valid := legacyEscrowTagged(command)
	if !valid {
		return false
	}
	_, migration, project, key := command.legacyEscrow()
	return project > 0 && legacyEscrowKey(migration) && legacyEscrowKey(key)
}

func decodeLegacyEscrow(raw []byte, out any) error {
	// Wire fields are lowercase ASCII. Reject aliases before encoding/json's
	// case-insensitive field matching can change an operation or its receipt.
	keys := json.NewDecoder(bytes.NewReader(raw))
	var walk func(int) error
	walk = func(depth int) error {
		if depth > 32 {
			return ErrUnavailable
		}
		token, e := keys.Token()
		if e != nil {
			return e
		}
		if delimiter, ok := token.(json.Delim); ok {
			seen := map[string]bool{}
			for keys.More() {
				if delimiter == '{' {
					key, e := keys.Token()
					name, ok := key.(string)
					if e != nil || !ok || seen[name] {
						return ErrUnavailable
					}
					for _, ch := range []byte(name) {
						if !(ch >= 'a' && ch <= 'z' || ch >= '0' && ch <= '9' || ch == '_') {
							return ErrUnavailable
						}
					}
					seen[name] = true
				}
				if e := walk(depth + 1); e != nil {
					return e
				}
			}
			if _, e := keys.Token(); e != nil {
				return e
			}
		}
		return nil
	}
	if e := walk(0); e != nil {
		return e
	}
	d := json.NewDecoder(bytes.NewReader(raw))
	d.DisallowUnknownFields()
	if e := d.Decode(out); e != nil {
		return e
	}
	if d.Decode(new(any)) != io.EOF {
		return ErrUnavailable
	}
	return nil
}

func parseLegacyEscrowReceipt(raw []byte, key string) (LegacyEscrowReceipt, bool) {
	var w struct {
		Status         string  `json:"status"`
		Code           *string `json:"code"`
		OperationKey   string  `json:"operation_key"`
		JournalID      *int64  `json:"journal_id"`
		RemainingUnits *int64  `json:"remaining_units"`
		Revision       *int64  `json:"revision"`
	}
	if decodeLegacyEscrow(raw, &w) != nil || w.Status == "" || w.OperationKey != key || w.RemainingUnits == nil || *w.RemainingUnits < 0 || w.Revision == nil || *w.Revision <= 0 || (w.JournalID != nil && *w.JournalID <= 0) {
		return LegacyEscrowReceipt{}, false
	}
	switch w.Status {
	case "posted":
		if w.Code != nil {
			return LegacyEscrowReceipt{}, false
		}
	case "submitted":
		if w.Code != nil || w.JournalID != nil {
			return LegacyEscrowReceipt{}, false
		}
	case "rejected":
		if w.Code == nil || *w.Code == "" || w.JournalID != nil {
			return LegacyEscrowReceipt{}, false
		}
	default:
		return LegacyEscrowReceipt{}, false
	}
	return LegacyEscrowReceipt{w.Status, w.Code, w.OperationKey, w.JournalID, *w.RemainingUnits, *w.Revision}, true
}

func legacyEscrowReceiptForKind(r LegacyEscrowReceipt, kind string) bool {
	if r.Status == "rejected" {
		return true
	}
	if kind == "submit" {
		return r.Status == "submitted"
	}
	return r.Status == "posted" && (kind != "payout" || r.JournalID != nil)
}

func (c *CoreClient) legacyEscrowCall(ctx context.Context, session, action string, payload any, mutating bool) ([]byte, error) {
	if c == nil || c.http == nil || session == "" || len(session) > 4096 {
		return nil, ErrForbidden
	}
	for _, ch := range []byte(session) {
		if ch == '\r' || ch == '\n' || ch == 0 {
			return nil, ErrForbidden
		}
	}
	b, e := json.Marshal(payload)
	if e != nil || len(b) > 16384 {
		return nil, ErrInvalid
	}
	req, e := http.NewRequestWithContext(ctx, http.MethodPost, "http://core/internal/bounties/v1/legacy-escrow/"+action, bytes.NewReader(b))
	if e != nil {
		return nil, ErrInvalid
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+c.serviceToken)
	req.Header.Set("X-Lmm-Protocol", "1")
	req.Header.Set("X-Lmm-User-Credential", session)
	ambiguous := ErrUnavailable
	if mutating {
		ambiguous = ErrUnknown
	}
	resp, e := c.http.Do(req)
	if e != nil {
		return nil, ambiguous
	}
	defer resp.Body.Close()
	switch resp.StatusCode {
	case http.StatusOK:
	case http.StatusBadRequest:
		return nil, ErrInvalid
	case http.StatusUnauthorized, http.StatusForbidden:
		return nil, ErrForbidden
	case http.StatusNotFound:
		return nil, ErrNotFound
	case http.StatusConflict:
		return nil, ErrConflict
	default:
		return nil, ambiguous
	}
	raw, e := io.ReadAll(io.LimitReader(resp.Body, 65537))
	if e != nil || len(raw) > 65536 {
		return nil, ambiguous
	}
	return raw, nil
}

func (c *CoreClient) legacyEscrowExecute(ctx context.Context, session string, command LegacyEscrowCommand) (LegacyEscrowReceipt, error) {
	if !validLegacyEscrowCommand(command) {
		return LegacyEscrowReceipt{}, ErrInvalid
	}
	kind, _, _, key := command.legacyEscrow()
	raw, e := c.legacyEscrowCall(ctx, session, kind, command, true)
	if e != nil {
		return LegacyEscrowReceipt{}, e
	}
	r, valid := parseLegacyEscrowReceipt(raw, key)
	if !valid || !legacyEscrowReceiptForKind(r, kind) {
		return LegacyEscrowReceipt{}, ErrUnknown
	}
	return r, nil
}

func (c *CoreClient) SubmitLegacyEscrow(ctx context.Context, session string, command LegacyEscrowSubmitCommand) (LegacyEscrowReceipt, error) {
	return c.legacyEscrowExecute(ctx, session, command)
}
func (c *CoreClient) PayoutLegacyEscrow(ctx context.Context, session string, command LegacyEscrowPayoutCommand) (LegacyEscrowReceipt, error) {
	return c.legacyEscrowExecute(ctx, session, command)
}
func (c *CoreClient) CloseLegacyEscrow(ctx context.Context, session string, command LegacyEscrowCloseCommand) (LegacyEscrowReceipt, error) {
	return c.legacyEscrowExecute(ctx, session, command)
}

// QueryLegacyEscrowForCommand only reads the original operation. Neither a
// missing result nor an unavailable query causes an execution or a new key.
func (c *CoreClient) QueryLegacyEscrowForCommand(ctx context.Context, session string, command LegacyEscrowCommand) (LegacyEscrowObservation, error) {
	if !validLegacyEscrowCommand(command) {
		return LegacyEscrowObservation{}, ErrInvalid
	}
	kind, migration, project, key := command.legacyEscrow()
	tagged, _ := legacyEscrowTagged(command)
	payload := struct {
		MigrationID     string `json:"migration_id"`
		SourceProjectID int64  `json:"source_project_id"`
		OperationKey    string `json:"operation_key"`
		ExpectedCommand any    `json:"expected_command"`
	}{migration, project, key, tagged}
	raw, e := c.legacyEscrowCall(ctx, session, "query", payload, false)
	if e != nil {
		return LegacyEscrowObservation{}, e
	}
	var wire struct {
		Command     json.RawMessage `json:"command"`
		ActorUserID int64           `json:"actor_user_id"`
		Receipt     json.RawMessage `json:"receipt"`
	}
	if decodeLegacyEscrow(raw, &wire) != nil || wire.ActorUserID <= 0 {
		return LegacyEscrowObservation{}, ErrUnavailable
	}
	var observed any
	switch command.(type) {
	case LegacyEscrowSubmitCommand:
		observed = new(struct {
			Kind string `json:"kind"`
			LegacyEscrowSubmitCommand
		})
	case LegacyEscrowPayoutCommand:
		observed = new(struct {
			Kind string `json:"kind"`
			LegacyEscrowPayoutCommand
		})
	case LegacyEscrowCloseCommand:
		observed = new(struct {
			Kind string `json:"kind"`
			LegacyEscrowCloseCommand
		})
	}
	if decodeLegacyEscrow(wire.Command, observed) != nil {
		return LegacyEscrowObservation{}, ErrUnavailable
	}
	expected, _ := json.Marshal(tagged)
	actual, _ := json.Marshal(observed)
	if !bytes.Equal(expected, actual) {
		return LegacyEscrowObservation{}, ErrConflict
	}
	r, valid := parseLegacyEscrowReceipt(wire.Receipt, key)
	if !valid || !legacyEscrowReceiptForKind(r, kind) {
		return LegacyEscrowObservation{}, ErrUnavailable
	}
	return LegacyEscrowObservation{command, wire.ActorUserID, r}, nil
}
