package bounties

import (
	"context"
	"encoding/json"
	"strconv"
	"time"
)

type LedgerEntry struct {
	ID                 string  `json:"id"`
	ProjectID          string  `json:"project_id"`
	ChallengeID        string  `json:"challenge_id"`
	UserID             string  `json:"user_id"`
	CounterpartyUserID string  `json:"counterparty_user_id"`
	Kind               string  `json:"kind"`
	Quota              int64   `json:"quota"`
	Note               string  `json:"note"`
	JournalIDs         []int64 `json:"journal_ids"`
	CreatedAt          int64   `json:"created_at"`
}

func projectView(tx Tx, p Project) (map[string]any, error) {
	v, ok := tx.(ViewTx)
	if !ok {
		return nil, ErrUnavailable
	}
	m := map[string]any{}
	raw, _ := json.Marshal(p)
	json.Unmarshal(raw, &m)
	m["owner_user_id"] = webUserID(p.OwnerUserID)
	m["reward_quota"] = p.RewardCredit
	m["net_reward_quota"] = p.NetRewardCredit
	m["escrow_quota"] = p.EscrowCredit
	m["platform_fee_quota"] = int64(0)
	if p.NetRewardCredit > 0 {
		m["platform_fee_quota"] = (p.RewardCredit - p.NetRewardCredit) * int64(p.RewardSlots)
	}
	total, e := v.DistinctParticipants(p.ID)
	if e != nil {
		return nil, e
	}
	m["participant_count"] = total
	var active int64
	for _, status := range []string{"accepted", "submitted", "approved", "rejected", "withdrawn", "cancelled"} {
		n, e := v.Count("challenges", Query{ProjectID: p.ID, Status: status})
		if e != nil {
			return nil, e
		}
		m[status+"_challenge_count"] = n
		if status == "accepted" || status == "submitted" {
			active += n
		}
	}
	m["active_challenge_count"] = active
	n, e := v.Count("disputes", Query{ProjectID: p.ID, Status: "open"})
	if e != nil {
		return nil, e
	}
	m["open_dispute_count"] = n
	appeals, ends, e := v.Appeals(p.ID, time.Now().Unix())
	if e != nil {
		return nil, e
	}
	m["appealable_challenge_count"] = appeals
	m["appeal_window_ends_at"] = ends
	sum, n, e := v.Ratings(p.OwnerUserID, true)
	if e != nil {
		return nil, e
	}
	m["owner_rating_count"] = n
	avg := float64(0)
	if n > 0 {
		avg = float64(sum) / float64(n)
	}
	m["owner_rating_average"] = avg
	hearts, e := v.Thanks(p.OwnerUserID)
	if e != nil {
		return nil, e
	}
	m["owner_thank_heart_count"] = hearts
	return m, nil
}
func challengeView(tx Tx, ch Challenge) (map[string]any, error) {
	raw, _ := json.Marshal(ch)
	m := map[string]any{}
	json.Unmarshal(raw, &m)
	m["participant_user_id"] = webUserID(ch.ParticipantUserID)
	m["reward_quota"] = ch.RewardCredit
	v, ok := tx.(ViewTx)
	if !ok {
		return nil, ErrUnavailable
	}
	tips, e := v.Sum("notifications", Query{ProjectID: ch.ProjectID, ParticipantUserID: ch.ParticipantUserID, ChallengeID: ch.ID, TipsOnly: true})
	if e != nil {
		return nil, e
	}
	m["tip_quota"] = tips
	ch.TipCredit = tips
	var p Project
	if e = tx.Get("projects", ch.ProjectID, &p); e != nil {
		return nil, e
	}
	m["project_title"] = p.Title
	m["repository_url"] = p.RepositoryURL
	ds, e := v.All("disputes", Query{ProjectID: ch.ProjectID, ChallengeID: ch.ID})
	if e != nil {
		return nil, e
	}
	for _, raw := range ds {
		var d Dispute
		if json.Unmarshal(raw, &d) == nil && d.ChallengeID == ch.ID {
			m["dispute"] = disputeView(d, p, ch)
			if d.Status == "open" {
				break
			}
		}
	}
	return m, nil
}
func disputeView(d Dispute, p Project, ch Challenge) map[string]any {
	m := map[string]any{}
	raw, _ := json.Marshal(d)
	json.Unmarshal(raw, &m)
	against := d.OwnerUserID
	if d.OpenedByUserID == d.OwnerUserID {
		against = d.ParticipantUserID
	}
	m["against_user_id"] = webUserID(against)
	m["opened_by_user_id"] = webUserID(d.OpenedByUserID)
	m["resolved_by_user_id"] = webUserID(d.ResolvedByUserID)
	m["reason"] = d.Reason
	m["statement"] = d.Statement
	m["resolution"] = d.Resolution
	m["project_title_snapshot"] = d.ProjectSnapshot.Title
	m["repository_url_snapshot"] = d.ProjectSnapshot.RepositoryURL
	m["project_rules_snapshot"] = d.ProjectSnapshot.Rules
	m["project_escrow_quota_snapshot"] = d.ProjectSnapshot.EscrowCredit
	m["challenge_status_snapshot"] = d.ChallengeSnapshot.Status
	m["issue_url_snapshot"] = d.ChallengeSnapshot.IssueURL
	m["pull_request_url_snapshot"] = d.ChallengeSnapshot.PullRequestURL
	m["submission_note_snapshot"] = d.ChallengeSnapshot.SubmissionNote
	m["review_note_snapshot"] = d.ChallengeSnapshot.ReviewNote
	m["reward_quota_snapshot"] = d.ChallengeSnapshot.RewardCredit
	m["tip_quota_snapshot"] = d.TipCreditSnapshot
	m["tip_quota"] = ch.TipCredit
	m["owner_rating_score_snapshot"] = d.ChallengeSnapshot.OwnerRatingScore
	m["owner_rating_comment_snapshot"] = d.ChallengeSnapshot.OwnerRatingComment
	m["contributor_rating_score_snapshot"] = d.ChallengeSnapshot.ContributorRatingScore
	m["contributor_rating_comment_snapshot"] = d.ChallengeSnapshot.ContributorRatingComment
	m["project_title"] = p.Title
	m["repository_url"] = p.RepositoryURL
	m["project_rules"] = p.Rules
	m["current_project_escrow_quota"] = p.EscrowCredit
	m["challenge_status"] = ch.Status
	m["issue_url"] = ch.IssueURL
	m["pull_request_url"] = ch.PullRequestURL
	m["submission_note"] = ch.SubmissionNote
	m["review_note"] = ch.ReviewNote
	m["reward_quota"] = ch.RewardCredit
	m["owner_rating_score"] = ch.OwnerRatingScore
	m["owner_rating_comment"] = ch.OwnerRatingComment
	m["contributor_rating_score"] = ch.ContributorRatingScore
	m["contributor_rating_comment"] = ch.ContributorRatingComment
	m["live_evidence_changed"] = p.Title != d.ProjectSnapshot.Title || p.RepositoryURL != d.ProjectSnapshot.RepositoryURL || p.Rules != d.ProjectSnapshot.Rules || p.EscrowCredit != d.ProjectSnapshot.EscrowCredit || ch.Status != d.ChallengeSnapshot.Status || ch.IssueURL != d.ChallengeSnapshot.IssueURL || ch.PullRequestURL != d.ChallengeSnapshot.PullRequestURL || ch.SubmissionNote != d.ChallengeSnapshot.SubmissionNote || ch.ReviewNote != d.ChallengeSnapshot.ReviewNote || ch.RewardCredit != d.ChallengeSnapshot.RewardCredit || ch.TipCredit != d.TipCreditSnapshot || ch.OwnerRatingScore != d.ChallengeSnapshot.OwnerRatingScore || ch.OwnerRatingComment != d.ChallengeSnapshot.OwnerRatingComment || ch.ContributorRatingScore != d.ChallengeSnapshot.ContributorRatingScore || ch.ContributorRatingComment != d.ChallengeSnapshot.ContributorRatingComment
	return m
}
func (s *Service) Board(ctx context.Context, page, size int) (map[string]any, error) {
	r, e := s.PublicBoard(ctx, page, size)
	if e != nil {
		return nil, e
	}
	items := []map[string]any{}
	e = s.Store.Within(ctx, func(tx Tx) error {
		for _, raw := range r.Items {
			var p Project
			if e := json.Unmarshal(raw, &p); e != nil {
				return e
			}
			v, e := projectView(tx, p)
			if e != nil {
				return e
			}
			items = append(items, v)
		}
		return nil
	})
	return map[string]any{"items": items, "total": r.Total, "page": r.Page, "page_size": r.PageSize}, e
}
func (s *Service) Detail(ctx context.Context, session, projectID string) (map[string]any, error) {
	viewer := ""
	if session != "" {
		a, e := s.Funds.Authorize(ctx, session)
		if e == nil {
			viewer = a.UserID
		}
	}
	var out map[string]any
	e := s.Store.Within(ctx, func(tx Tx) error {
		var p Project
		if e := tx.Get("projects", projectID, &p); e != nil {
			return e
		}
		if p.Status == "deleted" || ((p.Status == "draft" || p.Status == "closed" || p.ArchivedAt > 0) && p.OwnerUserID != viewer) {
			return ErrNotFound
		}
		pv, e := projectView(tx, p)
		if e != nil {
			return e
		}
		challenges := []map[string]any{}
		ledger := []json.RawMessage{}
		v := tx.(ViewTx)
		if viewer != "" {
			mine, e := v.All("challenges", Query{ProjectID: p.ID, ParticipantUserID: viewer})
			if e != nil {
				return e
			}
			best := -1
			priority := map[string]int{"approved": 5, "submitted": 4, "accepted": 3, "rejected": 2}
			for _, raw := range mine {
				var ch Challenge
				json.Unmarshal(raw, &ch)
				if priority[ch.Status] > best {
					pv["viewer_challenge"], e = challengeView(tx, ch)
					if e != nil {
						return e
					}
					best = priority[ch.Status]
				}
			}
		}
		if viewer == p.OwnerUserID {
			rows, e := v.All("challenges", Query{ProjectID: p.ID})
			if e != nil {
				return e
			}
			for _, raw := range rows {
				var ch Challenge
				json.Unmarshal(raw, &ch)
				cv, e := challengeView(tx, ch)
				if e != nil {
					return e
				}
				challenges = append(challenges, cv)
			}
			ledger, e = v.All("ledger", Query{ProjectID: p.ID})
			if e != nil {
				return e
			}
		}
		out = map[string]any{"project": pv, "challenges": challenges, "ledger": ledger}
		return nil
	})
	return out, e
}

func (s *Service) BoardFor(ctx context.Context, session string, page, size int) (map[string]any, error) {
	out, e := s.Board(ctx, page, size)
	if e != nil || session == "" {
		return out, e
	}
	a, e := s.Funds.Authorize(ctx, session)
	if e != nil {
		return out, nil
	}
	items := out["items"].([]map[string]any)
	e = s.Store.Within(ctx, func(tx Tx) error {
		v := tx.(ViewTx)
		priority := map[string]int{"approved": 5, "submitted": 4, "accepted": 3, "rejected": 2}
		for _, m := range items {
			project, _ := m["id"].(string)
			rows, e := v.All("challenges", Query{ProjectID: project, ParticipantUserID: a.UserID})
			if e != nil {
				return e
			}
			best := -1
			for _, raw := range rows {
				var ch Challenge
				json.Unmarshal(raw, &ch)
				if priority[ch.Status] > best {
					m["viewer_challenge"], e = challengeView(tx, ch)
					if e != nil {
						return e
					}
					best = priority[ch.Status]
				}
			}
		}
		return nil
	})
	return out, e
}

func webUserID(s string) any {
	if n, e := strconv.ParseInt(s, 10, 64); e == nil && n > 0 {
		return n
	}
	return s
}
