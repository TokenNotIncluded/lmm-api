package bounties

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"strconv"
	"strings"
)

// SessionExtractor is supplied by the interactive host. It must reject host
// service tokens and conflicting credentials. No body/query user ID is trusted.
type SessionExtractor interface {
	Session(*http.Request) (string, error)
}
type SessionExtractorFunc func(*http.Request) (string, error)

func (f SessionExtractorFunc) Session(r *http.Request) (string, error) { return f(r) }

type HTTP struct {
	service  *Service
	sessions SessionExtractor
}

func NewHTTP(s *Service, extractor SessionExtractor) (http.Handler, error) {
	if s == nil || s.Store == nil || s.Funds == nil || extractor == nil {
		return nil, ErrInvalid
	}
	return &HTTP{s, extractor}, nil
}
func respond(w http.ResponseWriter, data any, e error, status int) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	if e != nil {
		status = 503
		switch e {
		case ErrInvalid:
			status = 400
		case ErrForbidden:
			status = 403
		case ErrNotFound:
			status = 404
		case ErrConflict:
			status = 409
		}
		w.WriteHeader(status)
		json.NewEncoder(w).Encode(map[string]any{"success": false, "code": "BOUNTY_ERROR", "message": e.Error(), "data": nil})
		return
	}
	if status == 0 {
		status = 200
	}
	w.WriteHeader(status)
	json.NewEncoder(w).Encode(map[string]any{"success": true, "data": data})
}
func numberQuery(r *http.Request, key string, def int) (int, error) {
	v, ok := r.URL.Query()[key]
	if !ok {
		return def, nil
	}
	if len(v) != 1 {
		return 0, ErrInvalid
	}
	n, e := strconv.Atoi(v[0])
	if e != nil {
		return 0, ErrInvalid
	}
	return n, nil
}

type webInput struct {
	RepositoryURL  string `json:"repository_url"`
	Title          string `json:"title"`
	Description    string `json:"description"`
	Rules          string `json:"rules"`
	RewardQuota    int64  `json:"reward_quota"`
	RewardCredit   int64  `json:"reward_credit"`
	RewardSlots    int    `json:"reward_slots"`
	GithubHandle   string `json:"github_handle"`
	IssueURL       string `json:"issue_url"`
	PullRequestURL string `json:"pull_request_url"`
	SubmissionNote string `json:"submission_note"`
	ReviewNote     string `json:"review_note"`
	RatingScore    int    `json:"rating_score"`
	RatingComment  string `json:"rating_comment"`
	Score          int    `json:"score"`
	Comment        string `json:"comment"`
	Quota          int64  `json:"quota"`
	Note           string `json:"note"`
	Reason         string `json:"reason"`
	Statement      string `json:"statement"`
	Action         string `json:"action"`
	Resolution     string `json:"resolution"`
}

func body(w http.ResponseWriter, r *http.Request) (webInput, error) {
	var in webInput
	d := json.NewDecoder(http.MaxBytesReader(w, r.Body, 24576))
	d.DisallowUnknownFields()
	e := d.Decode(&in)
	if e == io.EOF {
		return in, nil
	}
	if e != nil {
		return in, ErrInvalid
	}
	var extra any
	if d.Decode(&extra) != io.EOF {
		return in, ErrInvalid
	}
	return in, nil
}
func (h *HTTP) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	base := "/api/open-source-bounties"
	if r.URL.Path != base && !strings.HasPrefix(r.URL.Path, base+"/") {
		http.NotFound(w, r)
		return
	}
	path := strings.Trim(strings.TrimPrefix(r.URL.Path, base), "/")
	parts := strings.Split(path, "/")
	ctx := r.Context()
	if r.Method == http.MethodGet && path == "" {
		page, e := numberQuery(r, "page", 1)
		if e != nil {
			respond(w, nil, e, 0)
			return
		}
		size, e := numberQuery(r, "page_size", 20)
		if e != nil {
			respond(w, nil, e, 0)
			return
		}
		session, _ := h.sessions.Session(r)
		out, e := h.service.BoardFor(ctx, session, page, size)
		respond(w, out, e, 0)
		return
	}
	if r.Method == http.MethodGet && path == "config" {
		out, e := h.service.Config(ctx)
		respond(w, out, e, 0)
		return
	}
	if r.Method == http.MethodGet && len(parts) == 2 && parts[0] == "projects" {
		session, _ := h.sessions.Session(r)
		out, e := h.service.Detail(ctx, session, parts[1])
		respond(w, out, e, 0)
		return
	}
	session, e := h.sessions.Session(r)
	if e != nil || session == "" {
		respond(w, nil, ErrForbidden, 0)
		return
	}
	if len(parts) >= 2 && parts[0] == "operations" {
		if r.Method == http.MethodGet && len(parts) == 3 && parts[2] == "result" {
			op, e := h.service.webOperation(ctx, session, parts[1])
			if e != nil {
				respond(w, nil, e, 0)
				return
			}
			if op.State != "settled" {
				respond(w, nil, ErrConflict, 0)
				return
			}
			data, e := h.webResponse(ctx, op)
			respond(w, data, e, 0)
			return
		}

		if r.Method == http.MethodGet && len(parts) == 2 {
			op, e := h.service.webOperation(ctx, session, parts[1])
			if e != nil {
				respond(w, nil, e, 0)
				return
			}
			receipt, e := h.service.QueryOperation(ctx, session, op.ID)
			respond(w, receipt, e, 0)
			return
		}
		if r.Method == http.MethodPost && len(parts) == 3 && (parts[2] == "apply" || parts[2] == "recover") {
			if _, e := body(w, r); e != nil {
				respond(w, nil, e, 0)
				return
			}
			op, e := h.service.ApplyWeb(ctx, session, parts[1], parts[2] == "recover")
			data, viewErr := h.webResponse(ctx, op)
			if e == nil {
				e = viewErr
			}
			respond(w, data, e, webStatus(op))
			return
		}
		http.NotFound(w, r)
		return
	}
	limit, e := numberQuery(r, "limit", 50)
	if e != nil {
		respond(w, nil, e, 0)
		return
	}
	records := RecordRequest{Limit: limit}
	switch {
	case r.Method == http.MethodGet && path == "mine":
		records.Action = "mine"
		values := r.URL.Query()["archived"]
		if len(values) > 1 {
			respond(w, nil, ErrInvalid, 0)
			return
		}
		if len(values) == 1 {
			records.Archived, e = strconv.ParseBool(values[0])
			if e != nil {
				respond(w, nil, ErrInvalid, 0)
				return
			}
		}
	case r.Method == http.MethodGet && path == "accepted":
		records.Action = "accepted"
	case r.Method == http.MethodGet && path == "notifications":
		records.Action = "notifications"
	case r.Method == http.MethodPost && path == "notifications/read":
		records.Action = "notifications_read"
	case r.Method == http.MethodGet && path == "tips/received":
		records.Action = "tips_received"
	case r.Method == http.MethodPost && path == "tips/received/read":
		records.Action = "tips_read"
	case r.Method == http.MethodPost && len(parts) == 3 && parts[0] == "tips" && parts[2] == "thank":
		records.Action = "thank_tip"
		records.NotificationID = parts[1]
	case r.Method == http.MethodPost && len(parts) == 3 && parts[0] == "projects" && (parts[2] == "archive" || parts[2] == "unarchive"):
		records.Action = parts[2]
		records.ProjectID = parts[1]
	}
	if records.Action != "" {
		if r.Method != http.MethodGet {
			if _, e := body(w, r); e != nil {
				respond(w, nil, e, 0)
				return
			}
		}
		out, e := h.service.Records(ctx, session, records)
		if e == ErrForbidden && r.Method == http.MethodGet {
			respond(w, []any{}, nil, 0)
			return
		}
		var data any
		switch records.Action {
		case "mine":
			data, e = h.service.projectItems(ctx, out.Items, e)
		case "notifications", "tips_received":
			data = out.Items
		case "accepted":
			data, e = h.service.entityItems(ctx, "list_accepted", out.Items, e)
		case "thank_tip":
			data = out.Notification
		default:
			data = out.Project
		}
		respond(w, data, e, 0)
		return
	}
	if r.Method == http.MethodGet && (path == "accepted" || path == "disputes/mine" || path == "disputes/admin") {
		if len(r.URL.Query()["status"]) > 1 {
			respond(w, nil, ErrInvalid, 0)
			return
		}
		action := "list_accepted"
		if path != "accepted" {
			action = "list_disputes"
		}
		if path == "disputes/admin" {
			a, e := h.service.Funds.Authorize(ctx, session)
			if e != nil || !a.Admin {
				respond(w, nil, ErrForbidden, 0)
				return
			}
		}
		out, e := h.service.BuiltinInvoke(ctx, Invocation{Actor: Actor{Token: session}, Command: Command{Action: action}, Limit: limit, Status: r.URL.Query().Get("status")})
		if e == ErrForbidden && path != "disputes/admin" {
			respond(w, []any{}, nil, 0)
			return
		}
		items, e := h.service.entityItems(ctx, action, out.Items, e)
		respond(w, items, e, 0)
		return
	}
	in, e := body(w, r)
	if e != nil {
		respond(w, nil, e, 0)
		return
	}
	draft := Draft{Title: in.Title, Description: in.Description, Rules: in.Rules, RepositoryURL: in.RepositoryURL, RewardCredit: in.RewardQuota, RewardSlots: in.RewardSlots}
	if in.RewardCredit != 0 {
		if draft.RewardCredit != 0 && draft.RewardCredit != in.RewardCredit {
			respond(w, nil, ErrInvalid, 0)
			return
		}
		draft.RewardCredit = in.RewardCredit
	}
	cmd := Command{GithubHandle: in.GithubHandle, IssueURL: in.IssueURL, PullRequestURL: in.PullRequestURL, SubmissionNote: in.SubmissionNote, ReviewNote: in.ReviewNote, RatingScore: in.RatingScore, RatingComment: in.RatingComment, Credit: in.Quota, Reason: in.Reason, Statement: in.Statement, ResolveAction: in.Action, Resolution: in.Resolution}
	switch {
	case r.Method == http.MethodPost && path == "":
		cmd.Action = "create_draft"
	case r.Method == http.MethodPut && len(parts) == 2 && parts[0] == "projects":
		cmd.ProjectID = parts[1]
		if draft.RewardCredit == 0 && draft.RewardSlots == 0 {
			out, e := h.service.Records(ctx, session, RecordRequest{Action: "content", ProjectID: cmd.ProjectID, Draft: draft})
			data, e := h.service.projectResult(ctx, out.Project, e)
			respond(w, data, e, 0)
			return
		}
		cmd.Action = "update_draft"
	case r.Method == http.MethodDelete && len(parts) == 2 && parts[0] == "projects":
		cmd.Action = "delete_draft"
		cmd.ProjectID = parts[1]
	case r.Method == http.MethodPost && len(parts) == 3 && parts[0] == "projects":
		cmd.Action = parts[2]
		cmd.ProjectID = parts[1]
		switch cmd.Action {
		case "publish", "pause", "resume", "close", "accept", "submit":
		default:
			http.NotFound(w, r)
			return
		}
		if cmd.Action == "submit" {
			id, e := h.service.currentChallenge(ctx, session, cmd.ProjectID)
			if e != nil {
				respond(w, nil, e, 0)
				return
			}
			cmd.ChallengeID = id
		}
	case r.Method == http.MethodPost && len(parts) == 3 && parts[0] == "challenges":
		cmd.Action = parts[2]
		cmd.ChallengeID = parts[1]
		switch cmd.Action {
		case "withdraw", "cancel", "approve", "reject":
		case "tip":
			cmd.ReviewNote = in.Note
		case "rate-owner":
			cmd.Action = "rate_owner"
			cmd.RatingScore = in.Score
			cmd.RatingComment = in.Comment
		case "disputes":
			cmd.Action = "open_dispute"
		default:
			http.NotFound(w, r)
			return
		}
	case r.Method == http.MethodPost && len(parts) == 3 && parts[0] == "disputes" && parts[2] == "resolve":
		cmd.Action = "resolve_dispute"
		cmd.DisputeID = parts[1]
	default:
		http.NotFound(w, r)
		return
	}
	keys := r.Header.Values("Idempotency-Key")
	if len(keys) > 1 {
		respond(w, nil, ErrInvalid, 0)
		return
	}
	key := ""
	if len(keys) == 1 {
		key = keys[0]
	}
	if cmd.Action == "tip" && key == "" {
		respond(w, nil, ErrInvalid, 0)
		return
	}
	ids := r.Header.Values("X-Lmm-Operation-ID")
	if len(ids) > 1 {
		respond(w, nil, ErrInvalid, 0)
		return
	}
	if len(ids) == 1 {
		cmd.OperationID = ids[0]
	}
	op, e := h.service.PrepareWeb(ctx, session, key, Invocation{Command: cmd, Draft: draft})
	data, viewErr := h.webResponse(ctx, op)
	if e == nil {
		e = viewErr
	}
	respond(w, data, e, webStatus(op))
}
func webStatus(op WebOperation) int {
	if op.State == "settled" {
		return 200
	}
	return 202
}
func (h *HTTP) webResponse(ctx context.Context, op WebOperation) (any, error) {
	if op.State != "settled" {
		url := ""
		if op.Result.Receipt != nil {
			url = op.Result.Receipt.ConfirmationURL
		}
		return map[string]any{"state": op.State, "operation_id": op.ID, "receipt": op.Result.Receipt, "confirmation_url": url}, nil
	}
	r := op.Result
	action := op.Invocation.Command.Action
	if action == "delete_draft" {
		return nil, nil
	}
	if r.Project != nil {
		pv, e := h.service.projectResult(ctx, r.Project, nil)
		if e != nil {
			return nil, e
		}
		switch action {
		case "publish":
			return map[string]any{"project": pv, "charged_quota": r.Receipt.Credit}, nil
		case "close":
			return map[string]any{"project": pv, "refunded_quota": r.Receipt.Credit}, nil
		}
		return pv, nil
	}
	var out any
	e := h.service.Store.Within(ctx, func(tx Tx) error {
		if r.Challenge != nil {
			cv, e := challengeView(tx, *r.Challenge)
			if e != nil {
				return e
			}
			switch action {
			case "approve", "reject", "tip":
				m := map[string]any{"challenge": cv, "transferred_quota": r.Receipt.Credit}
				if r.Receipt.RemainingCredit != nil {
					m["remaining_quota"] = *r.Receipt.RemainingCredit
				}
				out = m
			default:
				out = cv
			}
			return nil
		}
		if r.Dispute != nil {
			var p Project
			var ch Challenge
			if e := tx.Get("projects", r.Dispute.ProjectID, &p); e != nil {
				return e
			}
			if e := tx.Get("challenges", r.Dispute.ChallengeID, &ch); e != nil {
				return e
			}
			tips, e := tx.(ViewTx).Sum("notifications", Query{ChallengeID: ch.ID, TipsOnly: true})
			if e != nil {
				return e
			}
			ch.TipCredit = tips
			out = disputeView(*r.Dispute, p, ch)
			return nil
		}
		out = r
		return nil
	})
	return out, e
}
func (s *Service) currentChallenge(ctx context.Context, session, project string) (string, error) {
	a, e := s.sessionActor(ctx, session)
	if e != nil {
		return "", e
	}
	id := ""
	e = s.Store.Within(ctx, func(tx Tx) error {
		v, ok := tx.(ViewTx)
		if !ok {
			return ErrUnavailable
		}
		rows, e := v.All("challenges", Query{ProjectID: project, ParticipantUserID: a.UserID, Status: "accepted"})
		if e != nil {
			return e
		}
		if len(rows) == 0 {
			return ErrNotFound
		}
		var ch Challenge
		if json.Unmarshal(rows[0], &ch) != nil {
			return ErrUnavailable
		}
		id = ch.ID
		return nil
	})
	return id, e
}
func (s *Service) projectResult(ctx context.Context, p *Project, e error) (any, error) {
	if e != nil || p == nil {
		return nil, e
	}
	var out map[string]any
	e = s.Store.Within(ctx, func(tx Tx) error { var e error; out, e = projectView(tx, *p); return e })
	return out, e
}
func (s *Service) projectItems(ctx context.Context, rows []json.RawMessage, e error) (any, error) {
	if e != nil {
		return nil, e
	}
	items := []map[string]any{}
	e = s.Store.Within(ctx, func(tx Tx) error {
		for _, raw := range rows {
			var p Project
			json.Unmarshal(raw, &p)
			v, e := projectView(tx, p)
			if e != nil {
				return e
			}
			items = append(items, v)
		}
		return nil
	})
	return items, e
}
func (s *Service) entityItems(ctx context.Context, kind string, rows []json.RawMessage, e error) (any, error) {
	if e != nil {
		return nil, e
	}
	items := []map[string]any{}
	e = s.Store.Within(ctx, func(tx Tx) error {
		for _, raw := range rows {
			if kind == "list_accepted" {
				var ch Challenge
				json.Unmarshal(raw, &ch)
				v, e := challengeView(tx, ch)
				if e != nil {
					return e
				}
				items = append(items, v)
			} else {
				var d Dispute
				json.Unmarshal(raw, &d)
				var p Project
				var ch Challenge
				if e := tx.Get("projects", d.ProjectID, &p); e != nil {
					return e
				}
				if e := tx.Get("challenges", d.ChallengeID, &ch); e != nil {
					return e
				}
				tips, e := tx.(ViewTx).Sum("notifications", Query{ChallengeID: ch.ID, TipsOnly: true})
				if e != nil {
					return e
				}
				ch.TipCredit = tips
				items = append(items, disputeView(d, p, ch))
			}
		}
		return nil
	})
	return items, e
}
