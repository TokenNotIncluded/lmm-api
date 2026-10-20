// Package bounties preserves extension business records. Financial authority,
// identity, confirmations and all funded transitions belong exclusively to Core.
// Rules adapted from the AGPL-3.0 project revision
// 3948a0a1ee1cefdd016a7aa45a11851545bb4b85, apps/api-go/model/open_source_bounty*.go.
package bounties

import (
	"context"
	"encoding/json"
	"errors"
	"time"
)

var (
	ErrInvalid     = errors.New("invalid bounty request")
	ErrForbidden   = errors.New("bounty access forbidden")
	ErrNotFound    = errors.New("bounty not found")
	ErrConflict    = errors.New("bounty state conflict")
	ErrUnavailable = errors.New("bounty authority unavailable")
)
var Actions = []string{"accept", "approve", "close", "cancel", "create_draft", "delete_draft", "get", "list", "list_accepted", "list_owned", "list_disputes", "open_dispute", "pause", "publish", "rate_owner", "reject", "resume", "resolve_dispute", "submit", "tip", "update_draft", "withdraw"}

type Actor struct {
	UserID string `json:"user_id"`
	Token  string `json:"-"`
	Admin  bool   `json:"admin"`
}
type Draft struct {
	Title         string `json:"title"`
	Description   string `json:"description"`
	Rules         string `json:"rules"`
	RepositoryURL string `json:"repository_url"`
	RewardCredit  int64  `json:"reward_credit"`
	RewardSlots   int    `json:"reward_slots"`
}
type Command struct {
	OperationID    string `json:"operation_id"`
	Action         string `json:"action"`
	ProjectID      string `json:"project_id"`
	ChallengeID    string `json:"challenge_id,omitempty"`
	DisputeID      string `json:"dispute_id,omitempty"`
	Confirmation   string `json:"confirmation,omitempty"`
	RewardCredit   int64  `json:"reward_credit,omitempty"`
	RewardSlots    int    `json:"reward_slots,omitempty"`
	RepositoryURL  string `json:"repository_url,omitempty"`
	Credit         int64  `json:"credit,omitempty"`
	Approve        bool   `json:"approve,omitempty"`
	GithubHandle   string `json:"github_handle,omitempty"`
	IssueURL       string `json:"issue_url,omitempty"`
	PullRequestURL string `json:"pull_request_url,omitempty"`
	SubmissionNote string `json:"submission_note,omitempty"`
	ReviewNote     string `json:"review_note,omitempty"`
	RatingScore    int    `json:"rating_score,omitempty"`
	RatingComment  string `json:"rating_comment,omitempty"`
	Reason         string `json:"reason,omitempty"`
	Statement      string `json:"statement,omitempty"`
	Resolution     string `json:"resolution,omitempty"`
	ResolveAction  string `json:"resolve_action,omitempty"`
}
type Receipt struct {
	FeeBasisPoints  *int64  `json:"fee_basis_points,omitempty"`
	RemainingCredit *int64  `json:"remaining_credit,omitempty"`
	ProjectRevision int64   `json:"project_revision,omitempty"`
	PublishedAt     int64   `json:"published_at,omitempty"`
	OperationID     string  `json:"operation_id"`
	ProjectID       string  `json:"project_id"`
	ChallengeID     string  `json:"challenge_id"`
	DisputeID       string  `json:"dispute_id"`
	State           string  `json:"state"`
	JournalIDs      []int64 `json:"journal_ids"`
	Credit          int64   `json:"credit"`
	ActorUserID     string  `json:"actor_user_id"`
	Confirmation    string  `json:"confirmation,omitempty"`
	ConfirmationURL string  `json:"confirmation_url,omitempty"`
	Status          string  `json:"status,omitempty"`
	EscrowCredit    int64   `json:"escrow_credit,omitempty"`
	RewardCredit    int64   `json:"reward_credit,omitempty"`
	FeeCredit       int64   `json:"fee_credit,omitempty"`
	Expires         int64   `json:"expires,omitempty"`
}

// Funds must authenticate the token in Core. Actor fields and Go records never
// authorize transfers. Preview binds the entire command; Execute is idempotent.
type Funds interface {
	Authorize(context.Context, string) (Actor, error)
	Preview(context.Context, string, Command) (Receipt, error)
	Execute(context.Context, string, Command) (Receipt, error)
}
type Invocation struct {
	Status   string
	Page     int
	PageSize int
	Actor    Actor
	Command  Command
	Draft    Draft
	After    string
	Limit    int
	Preview  bool
}
type Project struct {
	PlatformFeeRateBPS *int64 `json:"platform_fee_rate_bps,omitempty"`
	ClosedAt           int64  `json:"closed_at"`
	CreatedAt          int64  `json:"created_at"`
	ArchivedAt         int64  `json:"archived_at"`
	CoreRevision       int64  `json:"core_revision"`
	PublishedAt        int64  `json:"published_at"`
	EscrowCredit       int64  `json:"escrow_credit"`
	NetRewardCredit    int64  `json:"net_reward_credit"`
	ID                 string `json:"id"`
	OwnerUserID        string `json:"owner_user_id"`
	Draft
	Status    string `json:"status"`
	UpdatedAt int64  `json:"updated_at"`
}
type Challenge struct {
	TipCredit                int64   `json:"tip_credit"`
	OwnerRatedAt             int64   `json:"owner_rated_at"`
	ContributorRatedAt       int64   `json:"contributor_rated_at"`
	OwnerRatingOverturned    bool    `json:"owner_rating_overturned"`
	OwnerUserID              string  `json:"owner_user_id"`
	CreatedAt                int64   `json:"created_at"`
	AcceptedAt               int64   `json:"accepted_at"`
	SubmittedAt              int64   `json:"submitted_at"`
	ReviewedAt               int64   `json:"reviewed_at"`
	PaidAt                   int64   `json:"paid_at"`
	RejectedAt               int64   `json:"rejected_at"`
	RewardCredit             int64   `json:"reward_credit"`
	CoreRevision             int64   `json:"core_revision"`
	GithubHandle             string  `json:"github_handle"`
	IssueURL                 string  `json:"issue_url"`
	PullRequestURL           string  `json:"pull_request_url"`
	SubmissionNote           string  `json:"submission_note"`
	ReviewNote               string  `json:"review_note"`
	OwnerRatingScore         int     `json:"owner_rating_score"`
	OwnerRatingComment       string  `json:"owner_rating_comment"`
	ContributorRatingScore   int     `json:"contributor_rating_score"`
	ContributorRatingComment string  `json:"contributor_rating_comment"`
	ID                       string  `json:"id"`
	ProjectID                string  `json:"project_id"`
	ParticipantUserID        string  `json:"participant_user_id"`
	Status                   string  `json:"status"`
	Command                  Command `json:"evidence"`
	UpdatedAt                int64   `json:"updated_at"`
}
type Dispute struct {
	TipCreditSnapshot int64     `json:"tip_quota_snapshot"`
	Reason            string    `json:"reason"`
	Statement         string    `json:"statement"`
	Resolution        string    `json:"resolution"`
	CreatedAt         int64     `json:"created_at"`
	ResolvedAt        int64     `json:"resolved_at"`
	ResolvedByUserID  string    `json:"resolved_by_user_id"`
	CoreRevision      int64     `json:"core_revision"`
	ProjectSnapshot   Project   `json:"project_snapshot"`
	ChallengeSnapshot Challenge `json:"challenge_snapshot"`
	ID                string    `json:"id"`
	ProjectID         string    `json:"project_id"`
	ChallengeID       string    `json:"challenge_id"`
	OpenedByUserID    string    `json:"opened_by_user_id"`
	OwnerUserID       string    `json:"owner_user_id"`
	ParticipantUserID string    `json:"participant_user_id"`
	Status            string    `json:"status"`
	Command           Command   `json:"evidence"`
	UpdatedAt         int64     `json:"updated_at"`
}
type Result struct {
	Project   *Project          `json:"project,omitempty"`
	Challenge *Challenge        `json:"challenge,omitempty"`
	Dispute   *Dispute          `json:"dispute,omitempty"`
	Items     []json.RawMessage `json:"items,omitempty"`
	Receipt   *Receipt          `json:"receipt,omitempty"`
}
type Query struct {
	ChallengeID                                                   string
	Archived                                                      *bool
	TipsOnly                                                      bool
	RecipientUserID                                               string
	Status                                                        string
	ProjectID, OwnerUserID, ParticipantUserID, PartyUserID, After string
	Public                                                        bool
	Limit                                                         int
	Offset                                                        int
}
type Tx interface {
	Get(string, string, any) error
	Put(string, string, any) error
	List(string, Query) ([]json.RawMessage, error)
}
type Repository interface {
	Within(context.Context, func(Tx) error) error
}
type Service struct {
	Store Repository
	Funds Funds
	Now   func() time.Time
}
