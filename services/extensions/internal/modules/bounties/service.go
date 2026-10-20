package bounties

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"math"
	"net/url"
	"regexp"
	"strconv"
	"strings"
	"time"
)

var githubName = regexp.MustCompile(`^[A-Za-z0-9](?:[A-Za-z0-9_.-]{0,98}[A-Za-z0-9])?$`)

func validID(s string) bool { return len(s) > 0 && len(s) <= 128 && strings.TrimSpace(s) == s }
func repository(raw string) (string, error) {
	u, e := url.Parse(strings.TrimSpace(raw))
	if e != nil || u.Scheme != "https" || u.Host != "github.com" || u.User != nil || u.RawQuery != "" || u.Fragment != "" {
		return "", ErrInvalid
	}
	p := strings.Split(strings.Trim(u.Path, "/"), "/")
	if len(p) != 2 {
		return "", ErrInvalid
	}
	p[1] = strings.TrimSuffix(p[1], ".git")
	if !githubName.MatchString(p[0]) || !githubName.MatchString(p[1]) {
		return "", ErrInvalid
	}
	return "https://github.com/" + p[0] + "/" + p[1], nil
}
func normalize(d Draft) (Draft, error) {
	var e error
	d.Title = strings.TrimSpace(d.Title)
	d.Description = strings.TrimSpace(d.Description)
	d.Rules = strings.TrimSpace(d.Rules)
	d.RepositoryURL, e = repository(d.RepositoryURL)
	if e != nil || len(d.Title) < 4 || len(d.Title) > 120 || len(d.Description) < 20 || len(d.Description) > 2000 || len(d.Rules) < 20 || len(d.Rules) > 5000 || d.RewardCredit <= 0 || d.RewardSlots < 1 || d.RewardSlots > 100 || d.RewardCredit > math.MaxInt64/int64(d.RewardSlots) {
		return d, ErrInvalid
	}
	return d, nil
}
func evidence(raw, repo, kind string) (string, error) {
	if raw == "" {
		return "", nil
	}
	u, e := url.Parse(strings.TrimSpace(raw))
	if e != nil || u.Scheme != "https" || u.Host != "github.com" || u.User != nil || u.RawQuery != "" || u.Fragment != "" {
		return "", ErrInvalid
	}
	p := strings.Split(strings.Trim(u.Path, "/"), "/")
	if len(p) != 4 || p[2] != kind {
		return "", ErrInvalid
	}
	n, e := strconv.ParseInt(p[3], 10, 64)
	if e != nil || n <= 0 {
		return "", ErrInvalid
	}
	r, e := repository("https://github.com/" + p[0] + "/" + p[1])
	if e != nil || !strings.EqualFold(r, repo) {
		return "", ErrInvalid
	}
	return r + "/" + kind + "/" + p[3], nil
}

type operation struct {
	Hash   string `json:"hash"`
	Actor  string `json:"actor"`
	Result Result `json:"result"`
}

func (s *Service) BuiltinInvoke(ctx context.Context, in Invocation) (Result, error) {
	return s.invoke(ctx, in, false)
}

// RecoverOperation rebuilds the extension projection after an ambiguous Go
// commit. Core verifies the original normalized command before any local write.
// Recovery never calls Preview or Execute.
func (s *Service) RecoverOperation(ctx context.Context, session string, in Invocation) (Result, error) {
	in.Actor = Actor{Token: session}
	return s.invoke(ctx, in, true)
}
func (s *Service) invoke(ctx context.Context, in Invocation, recovering bool) (Result, error) {
	var out Result
	if s == nil || s.Store == nil || s.Funds == nil || in.Actor.Token == "" {
		return out, ErrForbidden
	}
	token := in.Actor.Token
	var actor Actor
	var authErr error
	if recovering || protectedAction(in.Command.Action) {
		actor, authErr = s.sessionActor(ctx, token)
	} else {
		actor, authErr = s.Funds.Authorize(ctx, token)
	}
	if authErr != nil {
		return out, authErr
	}
	if !validID(actor.UserID) {
		return out, ErrForbidden
	}
	actor.Token = token
	in.Actor = actor
	c := in.Command
	if recovering {
		if in.Preview {
			return out, ErrInvalid
		}
		switch c.Action {
		case "list", "get", "list_owned", "list_accepted", "list_disputes":
			return out, ErrInvalid
		}
	}
	if in.Preview && (c.Action == "create_draft" || c.Action == "update_draft") {
		return out, ErrInvalid
	}
	known := false
	for _, a := range Actions {
		if c.Action == a {
			known = true
		}
	}
	if !known {
		return out, ErrInvalid
	}
	in.Status = strings.TrimSpace(in.Status)
	if c.Action == "list_disputes" {
		switch in.Status {
		case "", "open", "resolved_paid", "resolved_denied":
		default:
			return out, ErrInvalid
		}
	} else if in.Status != "" {
		return out, ErrInvalid
	}
	maxLimit := 50
	if c.Action == "list_disputes" {
		maxLimit = 100
	}
	if in.PageSize != 0 {
		in.Limit = in.PageSize
	}
	if in.Limit == 0 {
		in.Limit = 50
	}
	if in.Page == 0 {
		in.Page = 1
	}
	if in.Limit < 1 || in.Limit > maxLimit || in.Page < 1 || in.Page-1 > math.MaxInt/in.Limit || in.After != "" && in.Page > 1 {
		return out, ErrInvalid
	}
	now := time.Now().Unix()
	if s.Now != nil {
		now = s.Now().Unix()
	}
	err := s.Store.Within(ctx, func(tx Tx) error {
		q := Query{Status: in.Status, After: in.After, Limit: in.Limit, Offset: (in.Page - 1) * in.Limit}
		switch c.Action {
		case "list":
			q.Public = true
			items, e := tx.List("projects", q)
			out.Items = items
			return e
		case "list_owned":
			q.OwnerUserID = in.Actor.UserID
			items, e := tx.List("projects", q)
			out.Items = items
			return e
		case "list_accepted":
			q.ParticipantUserID = in.Actor.UserID
			items, e := tx.List("challenges", q)
			out.Items = items
			return e
		case "list_disputes":
			if !in.Actor.Admin {
				q.PartyUserID = in.Actor.UserID
			}
			items, e := tx.List("disputes", q)
			out.Items = items
			return e
		}
		// Legacy tool calls identify reviews by challenge and resolutions by
		// dispute. Resolve relationships from persistent records after Session
		// authentication; no caller-supplied project identity is trusted.
		if c.DisputeID != "" && c.Action != "open_dispute" {
			var ref Dispute
			if e := tx.Get("disputes", c.DisputeID, &ref); e != nil {
				return e
			}
			if c.ProjectID != "" && c.ProjectID != ref.ProjectID || c.ChallengeID != "" && c.ChallengeID != ref.ChallengeID {
				return ErrConflict
			}
			c.ProjectID = ref.ProjectID
			c.ChallengeID = ref.ChallengeID
		}
		if c.ChallengeID != "" && c.Action != "accept" {
			var ref Challenge
			if e := tx.Get("challenges", c.ChallengeID, &ref); e != nil {
				return e
			}
			if c.ProjectID != "" && c.ProjectID != ref.ProjectID {
				return ErrConflict
			}
			c.ProjectID = ref.ProjectID
		}
		if !validID(c.ProjectID) {
			return ErrInvalid
		}
		var p Project
		e := tx.Get("projects", c.ProjectID, &p)
		if c.Action == "get" {
			if e != nil {
				return e
			}
			if p.Status == "deleted" {
				return ErrNotFound
			}
			if (p.Status == "draft" || p.Status == "closed") && p.OwnerUserID != in.Actor.UserID {
				return ErrForbidden
			}
			out.Project = &p
			return nil
		}
		if !validID(c.OperationID) {
			return ErrInvalid
		}
		// Persisted Core receipts make retries safe even after the local view advances.
		raw, _ := json.Marshal(struct {
			Command Command
			Draft   Draft
		}{c, in.Draft})
		sum := sha256.Sum256(raw)
		hash := hex.EncodeToString(sum[:])
		var prior operation
		if e := tx.Get("operations", c.OperationID, &prior); e == nil {
			if prior.Actor != in.Actor.UserID || prior.Hash != hash {
				return ErrConflict
			}
			out = prior.Result
			return nil
		} else if e != ErrNotFound {
			return e
		}
		if recovering && (c.Action == "create_draft" || c.Action == "update_draft") {
			return ErrNotFound
		}
		if c.Action == "create_draft" {
			if e == nil {
				return ErrConflict
			}
			if e != ErrNotFound {
				return e
			}
			d, e := normalize(in.Draft)
			if e != nil {
				return e
			}
			p = Project{ID: c.ProjectID, OwnerUserID: in.Actor.UserID, Draft: d, Status: "draft", CreatedAt: now, UpdatedAt: now}
			out.Project = &p
			if e = tx.Put("projects", p.ID, p); e != nil {
				return e
			}
			return tx.Put("operations", c.OperationID, operation{hash, in.Actor.UserID, out})
		}
		if e != nil {
			return e
		}
		if p.Status == "deleted" {
			return ErrNotFound
		}
		owner := p.OwnerUserID == in.Actor.UserID
		switch c.Action {
		case "update_draft", "delete_draft":
			if !owner {
				return ErrForbidden
			}
			if p.Status != "draft" {
				return ErrConflict
			}
			if c.Action == "update_draft" {
				d, e := normalize(in.Draft)
				if e != nil {
					return e
				}
				p.Draft = d
			} else {
				var r Receipt
				if in.Preview {
					r, e = s.Funds.Preview(ctx, in.Actor.Token, c)
					if e != nil {
						return e
					}
					out.Receipt = &r
					return nil
				}
				if recovering {
					q, ok := s.Funds.(RecoveryFunds)
					if !ok {
						return ErrUnavailable
					}
					r, e = q.QueryForCommand(ctx, in.Actor.Token, c)
				} else {
					r, e = s.Funds.Execute(ctx, in.Actor.Token, c)
				}
				if e != nil {
					return e
				}
				if r.OperationID != c.OperationID || r.ProjectID != p.ID || r.ActorUserID != in.Actor.UserID || r.ChallengeID != c.ChallengeID || r.DisputeID != c.DisputeID {
					return ErrConflict
				}
				if recovering && pendingReceipt(r) {
					out.Receipt = &r
					return nil
				}
				if r.State != "deleted" {
					return ErrConflict
				}
				r.Confirmation = ""
				out.Receipt = &r
				p.Status = "deleted"
			}
			p.UpdatedAt = now
			out.Project = &p
			if e = tx.Put("projects", p.ID, p); e != nil {
				return e
			}
			return tx.Put("operations", c.OperationID, operation{hash, in.Actor.UserID, out})
		}
		if s.Funds == nil {
			return ErrUnavailable
		}
		var ch Challenge
		var d Dispute
		if recovering && c.Action == "open_dispute" {
			if e := tx.Get("disputes", c.DisputeID, &d); e != nil && e != ErrNotFound {
				return e
			}
			if d.ID != "" && (d.ProjectID != p.ID || d.ChallengeID != c.ChallengeID) {
				return ErrConflict
			}
		}
		if c.ChallengeID != "" {
			if !validID(c.ChallengeID) {
				return ErrInvalid
			}
			e = tx.Get("challenges", c.ChallengeID, &ch)
			if c.Action != "accept" && e != nil {
				return e
			}
			if e == nil && ch.ProjectID != p.ID {
				return ErrConflict
			}
		}
		switch c.Action {
		case "publish", "pause", "resume", "close":
			if !owner {
				return ErrForbidden
			}
		case "accept":
			if owner {
				return ErrForbidden
			}
			if !validID(c.ChallengeID) {
				return ErrInvalid
			}
			if ch.ID != "" && (!recovering || ch.ParticipantUserID != in.Actor.UserID) {
				return ErrConflict
			}
			c.GithubHandle = strings.TrimPrefix(strings.TrimSpace(c.GithubHandle), "@")
			if !githubName.MatchString(c.GithubHandle) {
				return ErrInvalid
			}
		case "submit", "withdraw", "rate_owner":
			if ch.ParticipantUserID != in.Actor.UserID {
				return ErrForbidden
			}
		case "approve", "reject", "cancel", "tip":
			if !owner {
				return ErrForbidden
			}
		case "open_dispute":
			if !owner && ch.ParticipantUserID != in.Actor.UserID {
				return ErrForbidden
			}
			if !validID(c.DisputeID) {
				return ErrInvalid
			}
		case "resolve_dispute":
			if !in.Actor.Admin || owner || ch.ParticipantUserID == in.Actor.UserID {
				return ErrForbidden
			}
			if e = tx.Get("disputes", c.DisputeID, &d); e != nil {
				return e
			}
			if d.ProjectID != p.ID || d.ChallengeID != ch.ID {
				return ErrConflict
			}
		default:
			return ErrInvalid
		}
		if c.Action == "publish" {
			c.RewardCredit = p.RewardCredit
			c.RewardSlots = p.RewardSlots
			c.RepositoryURL = p.RepositoryURL
		}
		if c.Action == "submit" {
			var e error
			c.IssueURL, e = evidence(c.IssueURL, p.RepositoryURL, "issues")
			if e != nil {
				return e
			}
			c.PullRequestURL, e = evidence(c.PullRequestURL, p.RepositoryURL, "pull")
			if e != nil || c.IssueURL == "" && c.PullRequestURL == "" || len(c.SubmissionNote) > 2000 {
				return ErrInvalid
			}
		}
		if c.Action == "approve" || c.Action == "reject" || c.Action == "rate_owner" {
			if c.RatingScore < 1 || c.RatingScore > 5 || len(strings.TrimSpace(c.RatingComment)) < 2 || len(c.RatingComment) > 1000 || len(c.ReviewNote) > 2000 {
				return ErrInvalid
			}
		}
		if c.Action == "tip" {
			c.ReviewNote = strings.TrimSpace(c.ReviewNote)
			if c.Credit <= 0 || c.Credit > 1_000_000_000 || len(c.ReviewNote) > 500 {
				return ErrInvalid
			}
		}
		if c.Action == "open_dispute" {
			switch c.Reason {
			case "merged_but_unpaid", "requirements_met_but_rejected", "misleading_requirements", "abusive_conduct", "other":
			default:
				return ErrInvalid
			}
			if len(strings.TrimSpace(c.Statement)) < 20 || len(c.Statement) > 5000 {
				return ErrInvalid
			}
		}
		if c.Action == "resolve_dispute" {
			if c.ResolveAction != "pay" && c.ResolveAction != "deny" || len(strings.TrimSpace(c.Resolution)) < 10 || len(c.Resolution) > 5000 {
				return ErrInvalid
			}
		}
		var receipt Receipt
		if in.Preview {
			receipt, e = s.Funds.Preview(ctx, in.Actor.Token, c)
			if e != nil {
				return e
			}
			out.Receipt = &receipt
			return nil
		}
		if recovering {
			q, ok := s.Funds.(RecoveryFunds)
			if !ok {
				return ErrUnavailable
			}
			receipt, e = q.QueryForCommand(ctx, in.Actor.Token, c)
		} else {
			receipt, e = s.Funds.Execute(ctx, in.Actor.Token, c)
		}
		if e != nil {
			return e
		}
		if receipt.OperationID != c.OperationID || receipt.ActorUserID != in.Actor.UserID || receipt.ProjectID != p.ID || receipt.ChallengeID != c.ChallengeID || receipt.DisputeID != c.DisputeID || receipt.State == "" {
			return ErrConflict
		}
		if recovering && pendingReceipt(receipt) {
			out.Receipt = &receipt
			return nil
		}
		// Receipts retain their original revision. Recovery may fill a missing
		// entity while a later project/claim/dispute projection stays intact.
		receipt.Confirmation = ""
		c.Confirmation = ""
		out.Receipt = &receipt
		projectCurrent := receipt.ProjectRevision >= p.CoreRevision
		if p.PublishedAt == 0 && receipt.PublishedAt > 0 {
			p.PublishedAt = receipt.PublishedAt
		}
		if projectCurrent {
			if receipt.Status != "" {
				p.Status = receipt.Status
				if (p.Status == "closed" || p.Status == "completed") && p.ClosedAt == 0 {
					p.ClosedAt = now
				}
			}
			if receipt.FeeBasisPoints != nil {
				p.PlatformFeeRateBPS = receipt.FeeBasisPoints
			}
			p.EscrowCredit = receipt.EscrowCredit
			p.NetRewardCredit = receipt.RewardCredit
			p.CoreRevision = receipt.ProjectRevision
			p.UpdatedAt = now
		}
		if e = tx.Put("projects", p.ID, p); e != nil {
			return e
		}
		switch c.Action {
		case "publish", "pause", "resume", "close":
			if projectCurrent {
				p.Status = receipt.State
				if c.Action == "publish" && p.PublishedAt == 0 {
					p.PublishedAt = now
				}
				p.UpdatedAt = now
				if e = tx.Put("projects", p.ID, p); e != nil {
					return e
				}
			}
			out.Project = &p
		case "open_dispute":
			if d.ID == "" || receipt.ProjectRevision >= d.CoreRevision {
				d = Dispute{CoreRevision: receipt.ProjectRevision, Reason: c.Reason, Statement: c.Statement, CreatedAt: now, ProjectSnapshot: p, ChallengeSnapshot: ch, ID: c.DisputeID, ProjectID: p.ID, ChallengeID: ch.ID, OpenedByUserID: in.Actor.UserID, OwnerUserID: p.OwnerUserID, ParticipantUserID: ch.ParticipantUserID, Status: receipt.State, Command: c, UpdatedAt: now}
				if v, ok := tx.(ViewTx); ok {
					d.TipCreditSnapshot, e = v.Sum("notifications", Query{ChallengeID: ch.ID, TipsOnly: true})
					if e != nil {
						return e
					}
				}
				if e = tx.Put("disputes", d.ID, d); e != nil {
					return e
				}
			}
			out.Dispute = &d
		case "resolve_dispute":
			if receipt.State == "resolved_paid" && receipt.ProjectRevision >= ch.CoreRevision {
				if ch.Status == "rejected" && ch.OwnerRatingScore > 0 {
					ch.OwnerRatingOverturned = true
				}
				ch.Status = "approved"
				ch.PaidAt = now
				ch.CoreRevision = receipt.ProjectRevision
				ch.UpdatedAt = now
				if e = tx.Put("challenges", ch.ID, ch); e != nil {
					return e
				}
			}
			if receipt.ProjectRevision >= d.CoreRevision {
				d.Status = receipt.State
				d.Resolution = c.Resolution
				d.ResolvedAt = now
				d.ResolvedByUserID = in.Actor.UserID
				d.CoreRevision = receipt.ProjectRevision
				d.Command = c
				d.UpdatedAt = now
				if e = tx.Put("disputes", d.ID, d); e != nil {
					return e
				}
			}
			out.Dispute = &d
		default:
			if ch.ID == "" || receipt.ProjectRevision >= ch.CoreRevision {
				if c.Action == "accept" && ch.ID == "" {
					ch = Challenge{ID: c.ChallengeID, ProjectID: p.ID, ParticipantUserID: in.Actor.UserID, OwnerUserID: p.OwnerUserID, CreatedAt: now, AcceptedAt: now, RewardCredit: receipt.RewardCredit}
				}
				switch c.Action {
				case "accept":
					ch.GithubHandle = c.GithubHandle
				case "submit":
					ch.IssueURL = c.IssueURL
					ch.PullRequestURL = c.PullRequestURL
					ch.SubmissionNote = c.SubmissionNote
					ch.SubmittedAt = now
				case "approve", "reject":
					ch.ReviewNote = c.ReviewNote
					ch.OwnerRatedAt = now
					ch.OwnerRatingScore = c.RatingScore
					ch.ReviewedAt = now
					if c.Action == "approve" {
						ch.PaidAt = now
					} else {
						ch.RejectedAt = now
					}
					ch.OwnerRatingComment = c.RatingComment
				case "rate_owner":
					ch.ContributorRatedAt = now
					ch.ContributorRatingScore = c.RatingScore
					ch.ContributorRatingComment = c.RatingComment
				}
				ch.Status = receipt.State
				ch.CoreRevision = receipt.ProjectRevision
				ch.Command = c
				ch.UpdatedAt = now
				if e = tx.Put("challenges", ch.ID, ch); e != nil {
					return e
				}
			}
			out.Challenge = &ch
		}
		if e = s.projectNotification(tx, in.Actor, c, receipt, p, ch, now); e != nil {
			return e
		}
		return tx.Put("operations", c.OperationID, operation{hash, in.Actor.UserID, out})
	})
	return out, err
}
