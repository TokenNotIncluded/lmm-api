package model

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"regexp"
	"strings"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"gorm.io/gorm"
)

const openSourceBountyRESTTipOperation = "tip"

var (
	openSourceBountyIdempotencyKeyPattern = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._:-]{7,127}$`)
	errOpenSourceBountyRESTOperationRace  = errors.New("open-source bounty REST operation already exists")
)

type OpenSourceBountyRESTOperation struct {
	Id                 int    `json:"id"`
	UserId             int    `json:"user_id" gorm:"not null;uniqueIndex:idx_open_source_bounty_rest_operation,priority:1"`
	Operation          string `json:"operation" gorm:"type:varchar(64);not null;uniqueIndex:idx_open_source_bounty_rest_operation,priority:2"`
	IdempotencyKeyHash string `json:"-" gorm:"type:char(64);not null;uniqueIndex:idx_open_source_bounty_rest_operation,priority:3"`
	PayloadHash        string `json:"-" gorm:"type:char(64);not null"`
	ResultJson         string `json:"-" gorm:"type:text;not null;default:''"`
	CreatedAt          int64  `json:"created_at" gorm:"bigint;not null;index"`
	CompletedAt        int64  `json:"completed_at" gorm:"bigint;not null;default:0;index"`
}

func (OpenSourceBountyRESTOperation) TableName() string {
	return "open_source_bounty_rest_operations"
}

type OpenSourceBountyTipResult struct {
	Challenge        OpenSourceBountyChallenge `json:"challenge"`
	TransferredQuota int                       `json:"transferred_quota"`
	RemainingQuota   int                       `json:"remaining_quota"`
}

type openSourceBountyRESTOperationSpec struct {
	UserId             int
	Operation          string
	IdempotencyKeyHash string
	PayloadHash        string
}

func validateOpenSourceBountyIdempotencyKey(key string) error {
	if !openSourceBountyIdempotencyKeyPattern.MatchString(key) {
		return bountyError("OPEN_SOURCE_BOUNTY_INVALID_IDEMPOTENCY_KEY", "Idempotency-Key must contain 8 to 128 supported characters")
	}
	return nil
}

func hashOpenSourceBountyIdempotencyKey(key string) string {
	sum := sha256.Sum256([]byte(key))
	return hex.EncodeToString(sum[:])
}

func newOpenSourceBountyRESTTipOperationSpec(userId int, challengeId int, quota int, note string, idempotencyKey string) (*openSourceBountyRESTOperationSpec, error) {
	if err := validateOpenSourceBountyIdempotencyKey(idempotencyKey); err != nil {
		return nil, err
	}
	payloadHash, err := OpenSourceBountyMCPPayloadHash(struct {
		ChallengeId int    `json:"challenge_id"`
		Quota       int    `json:"quota"`
		Note        string `json:"note"`
	}{ChallengeId: challengeId, Quota: quota, Note: strings.TrimSpace(note)})
	if err != nil {
		return nil, err
	}
	return &openSourceBountyRESTOperationSpec{
		UserId: userId, Operation: openSourceBountyRESTTipOperation,
		IdempotencyKeyHash: hashOpenSourceBountyIdempotencyKey(idempotencyKey), PayloadHash: payloadHash,
	}, nil
}

func decodeOpenSourceBountyRESTTipResult(operation *OpenSourceBountyRESTOperation, spec *openSourceBountyRESTOperationSpec) (*OpenSourceBountyTipResult, error) {
	if operation.PayloadHash != spec.PayloadHash {
		return nil, bountyError("OPEN_SOURCE_BOUNTY_IDEMPOTENCY_MISMATCH", "Idempotency-Key was already used for a different bounty tip")
	}
	if operation.CompletedAt <= 0 || operation.ResultJson == "" {
		return nil, bountyError("OPEN_SOURCE_BOUNTY_IDEMPOTENCY_IN_PROGRESS", "the bounty tip is still being committed; retry with the same Idempotency-Key")
	}
	var result OpenSourceBountyTipResult
	if err := json.Unmarshal([]byte(operation.ResultJson), &result); err != nil {
		return nil, err
	}
	return &result, nil
}

func reserveOpenSourceBountyRESTOperationTx(tx *gorm.DB, spec *openSourceBountyRESTOperationSpec) (*OpenSourceBountyRESTOperation, *OpenSourceBountyTipResult, error) {
	var existing OpenSourceBountyRESTOperation
	err := lockForUpdate(tx).Where(
		"user_id = ? AND operation = ? AND idempotency_key_hash = ?",
		spec.UserId, spec.Operation, spec.IdempotencyKeyHash,
	).First(&existing).Error
	if err == nil {
		result, err := decodeOpenSourceBountyRESTTipResult(&existing, spec)
		return nil, result, err
	}
	if !errors.Is(err, gorm.ErrRecordNotFound) {
		return nil, nil, err
	}
	operation := &OpenSourceBountyRESTOperation{
		UserId: spec.UserId, Operation: spec.Operation, IdempotencyKeyHash: spec.IdempotencyKeyHash,
		PayloadHash: spec.PayloadHash, CreatedAt: common.GetTimestamp(),
	}
	if err := tx.Create(operation).Error; err != nil {
		lower := strings.ToLower(err.Error())
		if strings.Contains(lower, "unique") || strings.Contains(lower, "duplicate") {
			return nil, nil, errOpenSourceBountyRESTOperationRace
		}
		return nil, nil, err
	}
	return operation, nil, nil
}

func completeOpenSourceBountyRESTOperationTx(tx *gorm.DB, operation *OpenSourceBountyRESTOperation, result *OpenSourceBountyTipResult) error {
	encoded, err := json.Marshal(result)
	if err != nil {
		return err
	}
	completedAt := common.GetTimestamp()
	updated := tx.Model(&OpenSourceBountyRESTOperation{}).
		Where("id = ? AND completed_at = 0", operation.Id).
		Updates(map[string]any{"result_json": string(encoded), "completed_at": completedAt})
	if updated.Error != nil {
		return updated.Error
	}
	if updated.RowsAffected != 1 {
		return errors.New("failed to commit open-source bounty REST operation result")
	}
	return nil
}

func getOpenSourceBountyRESTTipResult(spec *openSourceBountyRESTOperationSpec) (*OpenSourceBountyTipResult, error) {
	var operation OpenSourceBountyRESTOperation
	if err := DB.Where(
		"user_id = ? AND operation = ? AND idempotency_key_hash = ?",
		spec.UserId, spec.Operation, spec.IdempotencyKeyHash,
	).First(&operation).Error; err != nil {
		return nil, err
	}
	return decodeOpenSourceBountyRESTTipResult(&operation, spec)
}
