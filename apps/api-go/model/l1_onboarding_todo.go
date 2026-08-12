package model

import (
	"errors"
	"net/url"
	"strings"
	"time"

	"github.com/QuantumNous/new-api/common"
	"gorm.io/gorm"
)

const (
	L1OnboardingStepCreateAPIKey            = "create_api_key"
	L1OnboardingStepInstallClient           = "install_client"
	L1OnboardingStepConfigureClient         = "configure_client"
	L1OnboardingStepFirstSuccessfulResponse = "first_successful_response"
	L1OnboardingStatusInProgress            = "in_progress"
	L1OnboardingStatusCompleted             = "completed"
	L1OnboardingProofInstallClient          = "client_heartbeat"
	L1OnboardingProofConfigureClient        = "client_configuration"
	L1OnboardingClientNameMaxLength         = 64
	L1OnboardingBaseURLMaxLength            = 256
)

var (
	ErrL1OnboardingNotEligible   = errors.New("L1 onboarding is only available to users with approved developer access")
	ErrL1OnboardingInvalidStep   = errors.New("invalid L1 onboarding step")
	ErrL1OnboardingOutOfOrder    = errors.New("L1 onboarding steps must be completed in order")
	ErrL1OnboardingProofRequired = errors.New("a server-verified onboarding proof is required")
	ErrL1OnboardingInvalidProof  = errors.New("invalid onboarding proof")
)

// L1OnboardingTodo stores only milestone timestamps and the owning user. It
// deliberately never stores an API key, base URL, credential, or request body.
// The first and last milestones are derived from durable server facts; the two
// client milestones are written only by the API-key-authenticated proof route.
type L1OnboardingTodo struct {
	Id                 int   `json:"id" gorm:"primaryKey"`
	UserId             int   `json:"user_id" gorm:"not null;uniqueIndex"`
	ClientInstalledAt  int64 `json:"client_installed_at" gorm:"not null;default:0"`
	ClientConfiguredAt int64 `json:"client_configured_at" gorm:"not null;default:0"`
	CompletedAt        int64 `json:"completed_at" gorm:"not null;default:0"`
	CreatedAt          int64 `json:"created_at" gorm:"not null;autoCreateTime"`
	UpdatedAt          int64 `json:"updated_at" gorm:"not null;autoUpdateTime"`
}

func (L1OnboardingTodo) TableName() string { return "l1_onboarding_todos" }

type L1OnboardingEligibility struct {
	Eligible               bool   `json:"eligible"`
	DeveloperAccessGranted bool   `json:"developer_access_granted"`
	TrustLevel             int    `json:"trust_level"`
	Reason                 string `json:"reason,omitempty"`
}

type L1OnboardingStepState struct {
	Id          string `json:"id"`
	Status      string `json:"status"`
	CompletedAt int64  `json:"completed_at,omitempty"`
}

type L1OnboardingTodoView struct {
	Eligibility L1OnboardingEligibility `json:"eligibility"`
	Status      string                  `json:"status"`
	CurrentStep string                  `json:"current_step,omitempty"`
	Steps       []L1OnboardingStepState `json:"steps"`
	CompletedAt int64                   `json:"completed_at,omitempty"`
}

type L1OnboardingProof struct {
	Step    string `json:"step"`
	Client  string `json:"client"`
	BaseURL string `json:"base_url"`
	Group   string `json:"group"`
}

func L1OnboardingStepIDs() []string {
	return []string{
		L1OnboardingStepCreateAPIKey,
		L1OnboardingStepInstallClient,
		L1OnboardingStepConfigureClient,
		L1OnboardingStepFirstSuccessfulResponse,
	}
}

func L1OnboardingEligibilityForUser(user *User) (L1OnboardingEligibility, error) {
	if user == nil {
		return L1OnboardingEligibility{}, gorm.ErrInvalidData
	}
	snapshot, err := GetFreshUserAccessSnapshot(user)
	if err != nil {
		return L1OnboardingEligibility{}, err
	}
	eligible := snapshot.TrustLevel.Level >= TrustLevelMinUser+1 || snapshot.DeveloperAccess.Granted
	result := L1OnboardingEligibility{
		Eligible:               eligible,
		DeveloperAccessGranted: snapshot.DeveloperAccess.Granted,
		TrustLevel:             snapshot.TrustLevel.Level,
	}
	if !eligible {
		result.Reason = "L1_REQUIRED"
	}
	return result, nil
}

func getL1OnboardingUser(userID int) (*User, L1OnboardingEligibility, error) {
	if userID <= 0 || DB == nil {
		return nil, L1OnboardingEligibility{}, gorm.ErrInvalidData
	}
	var user User
	if err := DB.Select("id", "role", "status", "created_at", "last_api_activity_at", "request_count", "trust_level_override", "console_activated_at").First(&user, "id = ?", userID).Error; err != nil {
		return nil, L1OnboardingEligibility{}, err
	}
	eligibility, err := L1OnboardingEligibilityForUser(&user)
	return &user, eligibility, err
}

func getOrCreateL1OnboardingTodo(userID int, now int64) (*L1OnboardingTodo, error) {
	var todo L1OnboardingTodo
	err := DB.Where("user_id = ?", userID).First(&todo).Error
	if err == nil {
		return &todo, nil
	}
	if !errors.Is(err, gorm.ErrRecordNotFound) {
		return nil, err
	}
	todo = L1OnboardingTodo{UserId: userID, CreatedAt: now, UpdatedAt: now}
	if err = DB.Create(&todo).Error; err != nil {
		// Another request may have created the singleton concurrently. Read it
		// back to keep retries idempotent across all supported SQL databases.
		if readErr := DB.Where("user_id = ?", userID).First(&todo).Error; readErr == nil {
			return &todo, nil
		}
		return nil, err
	}
	return &todo, nil
}

func activeL1OnboardingKeyExists(userID int) (bool, error) {
	var count int64
	err := DB.Model(&Token{}).Where("user_id = ? AND status = ?", userID, common.TokenStatusEnabled).Count(&count).Error
	return count > 0, err
}

func normalizeL1OnboardingProof(proof L1OnboardingProof) (L1OnboardingProof, error) {
	proof.Step = strings.TrimSpace(proof.Step)
	proof.Client = strings.TrimSpace(proof.Client)
	proof.BaseURL = strings.TrimSpace(proof.BaseURL)
	proof.Group = strings.TrimSpace(proof.Group)
	if proof.Step != L1OnboardingStepInstallClient && proof.Step != L1OnboardingStepConfigureClient {
		return L1OnboardingProof{}, ErrL1OnboardingInvalidStep
	}
	if len([]rune(proof.Client)) == 0 || len([]rune(proof.Client)) > L1OnboardingClientNameMaxLength {
		return L1OnboardingProof{}, ErrL1OnboardingInvalidProof
	}
	if proof.Step == L1OnboardingStepConfigureClient {
		if proof.BaseURL == "" || len([]rune(proof.BaseURL)) > L1OnboardingBaseURLMaxLength || proof.Group == "" {
			return L1OnboardingProof{}, ErrL1OnboardingInvalidProof
		}
		parsed, err := url.Parse(proof.BaseURL)
		if err != nil || (parsed.Scheme != "http" && parsed.Scheme != "https") || parsed.Host == "" || parsed.User != nil {
			return L1OnboardingProof{}, ErrL1OnboardingInvalidProof
		}
	}
	return proof, nil
}

func refreshL1OnboardingTodo(user *User, todo *L1OnboardingTodo, now int64) (L1OnboardingTodoView, error) {
	eligibility, err := L1OnboardingEligibilityForUser(user)
	if err != nil {
		return L1OnboardingTodoView{}, err
	}
	if !eligibility.Eligible {
		return L1OnboardingTodoView{
			Eligibility: eligibility,
			Status:      "unavailable",
			Steps:       []L1OnboardingStepState{},
		}, nil
	}
	keyComplete, err := activeL1OnboardingKeyExists(user.Id)
	if err != nil {
		return L1OnboardingTodoView{}, err
	}
	installComplete := keyComplete && todo.ClientInstalledAt > 0
	configureComplete := installComplete && todo.ClientConfiguredAt > 0
	firstResponseComplete := configureComplete && user.LastAPIActivityAt >= todo.ClientConfiguredAt && user.LastAPIActivityAt > 0

	if firstResponseComplete && todo.CompletedAt == 0 {
		todo.CompletedAt = now
	}
	if todo.CompletedAt > 0 && !firstResponseComplete {
		todo.CompletedAt = 0
	}
	if err := DB.Model(todo).Updates(map[string]any{
		"completed_at": todo.CompletedAt,
		"updated_at":   now,
	}).Error; err != nil {
		return L1OnboardingTodoView{}, err
	}

	steps := []L1OnboardingStepState{
		{Id: L1OnboardingStepCreateAPIKey, Status: stepStatus(keyComplete), CompletedAt: boolTimestamp(keyComplete, todo.CreatedAt)},
		{Id: L1OnboardingStepInstallClient, Status: stepStatus(installComplete), CompletedAt: boolTimestamp(installComplete, todo.ClientInstalledAt)},
		{Id: L1OnboardingStepConfigureClient, Status: stepStatus(configureComplete), CompletedAt: boolTimestamp(configureComplete, todo.ClientConfiguredAt)},
		{Id: L1OnboardingStepFirstSuccessfulResponse, Status: stepStatus(firstResponseComplete), CompletedAt: boolTimestamp(firstResponseComplete, user.LastAPIActivityAt)},
	}
	current := ""
	for _, step := range steps {
		if step.Status != "completed" {
			current = step.Id
			break
		}
	}
	status := L1OnboardingStatusInProgress
	if todo.CompletedAt > 0 {
		status = L1OnboardingStatusCompleted
	}
	return L1OnboardingTodoView{
		Eligibility: eligibility,
		Status:      status,
		CurrentStep: current,
		Steps:       steps,
		CompletedAt: todo.CompletedAt,
	}, nil
}

func stepStatus(complete bool) string {
	if complete {
		return "completed"
	}
	return "pending"
}

func boolTimestamp(complete bool, timestamp int64) int64 {
	if complete {
		return timestamp
	}
	return 0
}

// GetL1OnboardingTodo returns no checklist row for L0 users. This is an
// intentional authorization boundary, not merely a frontend visibility rule.
func GetL1OnboardingTodo(userID int) (*L1OnboardingTodoView, error) {
	user, eligibility, err := getL1OnboardingUser(userID)
	if err != nil {
		return nil, err
	}
	if !eligibility.Eligible {
		view := &L1OnboardingTodoView{Eligibility: eligibility, Status: "unavailable", Steps: []L1OnboardingStepState{}}
		return view, nil
	}
	todo, err := getOrCreateL1OnboardingTodo(userID, time.Now().Unix())
	if err != nil {
		return nil, err
	}
	view, err := refreshL1OnboardingTodo(user, todo, time.Now().Unix())
	if err != nil {
		return nil, err
	}
	return &view, nil
}

// RefreshL1OnboardingTodo never trusts a browser-provided completed flag. It
// only re-reads server-derived milestones and is safe to call repeatedly.
func RefreshL1OnboardingTodo(userID int) (*L1OnboardingTodoView, error) {
	return GetL1OnboardingTodo(userID)
}

// ApplyL1OnboardingProof records only a proof received through API-key auth.
// The proof contains no credential and the API key itself is never persisted.
func ApplyL1OnboardingProof(userID, tokenID int, proof L1OnboardingProof, now int64) (*L1OnboardingTodoView, error) {
	if userID <= 0 || tokenID <= 0 {
		return nil, ErrL1OnboardingProofRequired
	}
	proof, err := normalizeL1OnboardingProof(proof)
	if err != nil {
		return nil, err
	}
	user, eligibility, err := getL1OnboardingUser(userID)
	if err != nil {
		return nil, err
	}
	if !eligibility.Eligible {
		return nil, ErrL1OnboardingNotEligible
	}
	var token Token
	if err := DB.Select("id", "user_id", "status", "group", "auto_groups").First(&token, "id = ? AND user_id = ?", tokenID, userID).Error; err != nil {
		return nil, ErrL1OnboardingProofRequired
	}
	if token.Status != common.TokenStatusEnabled {
		return nil, ErrL1OnboardingProofRequired
	}
	todo, err := getOrCreateL1OnboardingTodo(userID, now)
	if err != nil {
		return nil, err
	}
	current, err := refreshL1OnboardingTodo(user, todo, now)
	if err != nil {
		return nil, err
	}
	switch proof.Step {
	case L1OnboardingStepInstallClient:
		if todo.ClientInstalledAt > 0 {
			return &current, nil
		}
		if current.CurrentStep != L1OnboardingStepInstallClient {
			return nil, ErrL1OnboardingOutOfOrder
		}
		if todo.ClientInstalledAt == 0 {
			todo.ClientInstalledAt = now
			if err := DB.Model(todo).Updates(map[string]any{"client_installed_at": now, "updated_at": now}).Error; err != nil {
				return nil, err
			}
		}
	case L1OnboardingStepConfigureClient:
		if todo.ClientConfiguredAt > 0 {
			return &current, nil
		}
		if current.CurrentStep != L1OnboardingStepConfigureClient {
			return nil, ErrL1OnboardingOutOfOrder
		}
		if token.Group == "" && token.AutoGroups == "" {
			return nil, ErrL1OnboardingInvalidProof
		}
		groupMatches := token.Group == proof.Group
		if token.Group == "auto" {
			groups, groupErr := token.GetAutoGroups()
			if groupErr != nil {
				return nil, ErrL1OnboardingInvalidProof
			}
			for _, group := range groups {
				if group == proof.Group {
					groupMatches = true
					break
				}
			}
		}
		if !groupMatches {
			return nil, ErrL1OnboardingInvalidProof
		}
		if todo.ClientConfiguredAt == 0 {
			todo.ClientConfiguredAt = now
			if err := DB.Model(todo).Updates(map[string]any{"client_configured_at": now, "updated_at": now}).Error; err != nil {
				return nil, err
			}
		}
	}
	view, err := refreshL1OnboardingTodo(user, todo, now)
	if err != nil {
		return nil, err
	}
	return &view, nil
}
