package bounties

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"time"
)

type FeeLimits struct {
	RewardSlotsMin int64 `json:"reward_slots_min"`
	RewardSlotsMax int64 `json:"reward_slots_max"`
	TipCreditMin   int64 `json:"tip_credit_min"`
}
type FeeConfig struct {
	Enabled         bool        `json:"enabled"`
	Code            string      `json:"code,omitempty"`
	RateBasisPoints int64       `json:"rate_basis_points"`
	RatePercent     json.Number `json:"rate_percent"`
	Limits          FeeLimits   `json:"limits"`
}

type PublicFunds interface {
	PublicConfig(context.Context) (FeeConfig, error)
}
type Notification struct {
	ID              string `json:"id"`
	ProjectID       string `json:"project_id"`
	ChallengeID     string `json:"challenge_id"`
	RecipientUserID string `json:"participant_user_id"`
	SenderUserID    string `json:"sender_user_id"`
	Kind            string `json:"kind"`
	ProjectTitle    string `json:"project_title"`
	Quota           int64  `json:"quota"`
	Note            string `json:"note"`
	RecipientReadAt int64  `json:"recipient_read_at"`
	ThankedAt       int64  `json:"thanked_at"`
	CreatedAt       int64  `json:"created_at"`
}
type RecordRequest struct {
	Action, ProjectID, NotificationID, After, Status string
	Draft                                            Draft
	Archived                                         bool
	Page, PageSize, Limit                            int
}
type RecordResult struct {
	Project      *Project          `json:"project,omitempty"`
	Notification *Notification     `json:"notification,omitempty"`
	Items        []json.RawMessage `json:"items,omitempty"`
	Total        int64             `json:"total"`
	Page         int               `json:"page"`
	PageSize     int               `json:"page_size"`
}
type ViewTx interface {
	Appeals(string, int64) (int64, int64, error)
	DistinctParticipants(string) (int64, error)
	Thanks(string) (int64, error)
	All(string, Query) ([]json.RawMessage, error)
	Ratings(string, bool) (int64, int64, error)
	Count(string, Query) (int64, error)
	Sum(string, Query) (int64, error)
	MarkRead(string, bool, int64) error
}

func (s *Service) now() int64 {
	if s.Now != nil {
		return s.Now().Unix()
	}
	return time.Now().Unix()
}
func (s *Service) Config(ctx context.Context) (FeeConfig, error) {
	if s == nil || s.Funds == nil {
		return FeeConfig{}, ErrUnavailable
	}
	f, ok := s.Funds.(PublicFunds)
	if !ok {
		return FeeConfig{}, ErrUnavailable
	}
	return f.PublicConfig(ctx)
}

// Records only edits extension presentation metadata. Core receipts are the
// sole source of every notification amount, recipient, and funds projection.
func (s *Service) Records(ctx context.Context, session string, in RecordRequest) (RecordResult, error) {
	var out RecordResult
	if s == nil || s.Store == nil || s.Funds == nil {
		return out, ErrUnavailable
	}
	actor, e := s.Funds.Authorize(ctx, session)
	if e != nil {
		return out, e
	}
	if !validID(actor.UserID) {
		return out, ErrForbidden
	}
	now := s.now()
	if in.Limit == 0 {
		in.Limit = 50
	}
	if in.Limit < 1 || in.Limit > 100 {
		return out, ErrInvalid
	}
	e = s.Store.Within(ctx, func(tx Tx) error {
		view, ok := tx.(ViewTx)
		if !ok {
			return ErrUnavailable
		}
		switch in.Action {
		case "notifications", "tips_received":
			items, e := tx.List("notifications", Query{RecipientUserID: actor.UserID, TipsOnly: in.Action == "tips_received", Limit: in.Limit, After: in.After})
			out.Items = items
			return e
		case "notifications_read", "tips_read":
			return view.MarkRead(actor.UserID, in.Action == "tips_read", now)
		case "thank_tip":
			var n Notification
			if e := tx.Get("notifications", in.NotificationID, &n); e != nil {
				return e
			}
			if n.RecipientUserID != actor.UserID || n.Kind != "tip_transfer" {
				return ErrForbidden
			}
			if n.ThankedAt == 0 {
				n.ThankedAt = now
				n.RecipientReadAt = now
				if e := tx.Put("notifications", n.ID, n); e != nil {
					return e
				}
			}
			out.Notification = &n
			return nil
		case "accepted":
			items, e := view.All("challenges", Query{ParticipantUserID: actor.UserID})
			out.Items = items
			return e
		case "mine":
			q := Query{OwnerUserID: actor.UserID, Archived: &in.Archived, Limit: in.Limit, After: in.After}
			var e error
			out.Items, e = view.All("projects", q)
			return e
		}
		var p Project
		if !validID(in.ProjectID) {
			return ErrInvalid
		}
		if e := tx.Get("projects", in.ProjectID, &p); e != nil {
			return e
		}
		switch in.Action {
		case "archive", "unarchive":
			if p.OwnerUserID != actor.UserID {
				return ErrForbidden
			}
			if p.Status != "completed" && p.Status != "closed" {
				return ErrConflict
			}
			if in.Action == "archive" {
				if p.ArchivedAt == 0 {
					p.ArchivedAt = now
				}
			} else {
				p.ArchivedAt = 0
			}
		case "content":
			if p.OwnerUserID != actor.UserID && !actor.Admin {
				return ErrForbidden
			}
			if p.Status != "published" && p.Status != "paused" {
				return ErrConflict
			}
			d := in.Draft
			d.RepositoryURL = p.RepositoryURL
			d.RewardCredit = p.RewardCredit
			d.RewardSlots = p.RewardSlots
			d, e := normalize(d)
			if e != nil {
				return e
			}
			p.Title = d.Title
			p.Description = d.Description
			p.Rules = d.Rules
		default:
			return ErrInvalid
		}
		p.UpdatedAt = now
		if e := tx.Put("projects", p.ID, p); e != nil {
			return e
		}
		out.Project = &p
		return nil
	})
	return out, e
}
func (s *Service) PublicBoard(ctx context.Context, page, pageSize int) (RecordResult, error) {
	var out RecordResult
	if s == nil || s.Store == nil {
		return out, ErrUnavailable
	}
	if page == 0 {
		page = 1
	}
	if pageSize == 0 {
		pageSize = 20
	}
	if page < 1 || pageSize < 1 || pageSize > 50 || page-1 > int(^uint(0)>>1)/pageSize {
		return out, ErrInvalid
	}
	q := Query{Public: true, Limit: pageSize, Offset: (page - 1) * pageSize}
	e := s.Store.Within(ctx, func(tx Tx) error {
		view, ok := tx.(ViewTx)
		if !ok {
			return ErrUnavailable
		}
		var e error
		out.Total, e = view.Count("projects", q)
		if e != nil {
			return e
		}
		out.Items, e = view.All("projects", q)
		return e
	})
	out.Page = page
	out.PageSize = pageSize
	return out, e
}
func (s *Service) projectNotification(tx Tx, actor Actor, c Command, r Receipt, p Project, ch Challenge, now int64) error {
	if e := s.projectLedger(tx, c, r, p, ch, now); e != nil {
		return e
	}
	if r.Credit <= 0 {
		return nil
	}
	kind := ""
	switch c.Action {
	case "tip":
		kind = "tip_transfer"
	case "approve":
		kind = "reward_transfer"
	case "resolve_dispute":
		if c.ResolveAction == "pay" {
			kind = "dispute_reward_transfer"
		}
	}
	if kind == "" {
		return nil
	}
	var prior Notification
	if e := tx.Get("notifications", c.OperationID, &prior); e == nil {
		return nil
	} else if e != ErrNotFound {
		return e
	}
	if !validID(ch.ParticipantUserID) {
		return ErrConflict
	}
	n := Notification{ID: c.OperationID, ProjectID: p.ID, ChallengeID: ch.ID, RecipientUserID: ch.ParticipantUserID, SenderUserID: p.OwnerUserID, Kind: kind, ProjectTitle: p.Title, Quota: r.Credit, Note: c.ReviewNote, CreatedAt: now}
	if c.Action == "resolve_dispute" {
		n.Note = c.Resolution
	}
	return tx.Put("notifications", n.ID, n)
}

func (s *Service) projectLedger(tx Tx, c Command, r Receipt, p Project, ch Challenge, now int64) error {
	if len(r.JournalIDs) == 0 || r.Credit <= 0 {
		return nil
	}
	var entries []LedgerEntry
	switch c.Action {
	case "publish":
		entries = append(entries, LedgerEntry{Kind: "escrow_fund", Quota: r.EscrowCredit})
		if fee := r.Credit - r.EscrowCredit; fee > 0 {
			entries = append(entries, LedgerEntry{Kind: "platform_fee", Quota: fee})
		}
	case "close":
		entries = append(entries, LedgerEntry{Kind: "escrow_refund", Quota: r.Credit, CounterpartyUserID: p.OwnerUserID})
	case "tip":
		entries = append(entries, LedgerEntry{Kind: "tip_transfer", Quota: r.Credit, CounterpartyUserID: ch.ParticipantUserID, Note: c.ReviewNote})
	case "approve":
		entries = append(entries, LedgerEntry{Kind: "reward_transfer", Quota: r.Credit, CounterpartyUserID: ch.ParticipantUserID, Note: c.ReviewNote})
	case "resolve_dispute":
		if c.ResolveAction == "pay" {
			entries = append(entries, LedgerEntry{Kind: "dispute_reward_transfer", Quota: r.Credit, CounterpartyUserID: ch.ParticipantUserID, Note: c.Resolution})
		}
	}
	for _, row := range entries {
		sum := sha256.Sum256([]byte(c.OperationID + ":" + row.Kind))
		row.ID = hex.EncodeToString(sum[:])
		row.ProjectID = p.ID
		row.ChallengeID = ch.ID
		row.UserID = p.OwnerUserID
		row.CreatedAt = now
		row.JournalIDs = append([]int64(nil), r.JournalIDs...)
		var old LedgerEntry
		if e := tx.Get("ledger", row.ID, &old); e == nil {
			continue
		} else if e != ErrNotFound {
			return e
		}
		if e := tx.Put("ledger", row.ID, row); e != nil {
			return e
		}
	}
	return nil
}
