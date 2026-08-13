package model

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"regexp"
	"strings"
	"time"

	"github.com/QuantumNous/new-api/common"
	"gorm.io/gorm"
)

const (
	AssistantHistoryPrivacyNotice = "你与助手的对话不是私密通信，请勿发送个人信息、密码、API Key、Cookie 或其他凭证。敏感内容会被自动脱敏。"

	AssistantHistoryRoleUser      = "user"
	AssistantHistoryRoleAssistant = "assistant"
	AssistantHistoryRoleCard      = "secure_card"

	AssistantSecureCardTypeAPIKey = "api_key"

	assistantConversationTitleMaxRunes = 120
	assistantHistoryMessageMaxRunes    = 4000
	assistantHistoryPageMax            = 100
	assistantSecureCardPayloadMaxBytes = 16 * 1024
	assistantSecureCardDefaultLifetime = 10 * time.Minute
)

var (
	ErrAssistantConversationNotFound        = errors.New("assistant conversation not found")
	ErrAssistantHistoryForbidden            = errors.New("assistant conversation is not visible to this account")
	ErrAssistantConversationAlreadyArchived = errors.New("assistant conversation is already archived")
	ErrAssistantConversationNotArchived     = errors.New("assistant conversation is not archived")
	ErrAssistantSecureCardNotFound          = errors.New("assistant secure card not found")
	ErrAssistantSecureCardConsumed          = errors.New("assistant secure card has already been revealed")
	ErrAssistantSecureCardExpired           = errors.New("assistant secure card has expired")

	assistantHistoryAPIKeyPattern = regexp.MustCompile(`(?i)\b(?:sk|rk|pk|ak|tok|token|key|secret)[_-][a-z0-9._~+/-]{8,}\b`)
	assistantHistoryJWTPattern    = regexp.MustCompile(`\beyJ[a-zA-Z0-9_-]{10,}\.[a-zA-Z0-9_-]{10,}\.[a-zA-Z0-9_-]{10,}\b`)
	assistantHistoryEmailPattern  = regexp.MustCompile(`(?i)\b[a-z0-9.!#$%&'*+/=?^_` + "`" + `{|}~-]+@[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+\b`)
	assistantHistoryCookiePattern = regexp.MustCompile(`(?i)\b(cookie|set-cookie|session(?:[_ -]?id)?|csrf(?:[_ -]?token)?)\s*[:=：]\s*[^\s;,]+`)
	assistantHistoryBearerPattern = regexp.MustCompile(`(?i)\bbearer\s+[a-z0-9._~+/-]{6,}=*`)
	assistantHistorySecretPattern = regexp.MustCompile(`(?i)\b(password|passwd|pwd|api[ _-]?key|access[ _-]?token|refresh[ _-]?token|bearer|authorization|密碼|密码|密钥|令牌)\s*[:=：]\s*[^\s,;]+`)
	assistantHistoryURLSecret     = regexp.MustCompile(`(?i)([?&](?:api[_-]?key|access[_-]?token|token|password|passwd|secret)=)[^&#\s]+`)
	assistantHistoryPEMPrivateKey = regexp.MustCompile(`(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----`)
	assistantHistoryIPv4Pattern   = regexp.MustCompile(`\b(?:\d{1,3}\.){3}\d{1,3}\b`)
	assistantHistoryIPv6Pattern   = regexp.MustCompile(`[0-9A-Fa-f:]{2,39}`)
	assistantHistoryPhonePattern  = regexp.MustCompile(`(^|[^\w])((?:\+?86[\s-]?)?1[3-9]\d{9}|\+\d{1,3}(?:[\s.-]?\d{2,4}){2,4})([^\w]|$)`)
	assistantHistoryCardPattern   = regexp.MustCompile(`(^|[^0-9])([0-9][0-9 -]{11,22}[0-9])([^0-9]|$)`)
)

// AssistantConversation holds only redacted, support-oriented text.  Its
// owner is authoritative; clients never choose it when reading or continuing
// a conversation.
type AssistantConversation struct {
	Id                 int64  `json:"id" gorm:"primaryKey"`
	UserId             int    `json:"-" gorm:"not null;index:idx_assistant_conversation_user_updated,priority:1"`
	Title              string `json:"title" gorm:"type:varchar(255);not null"`
	LastMessagePreview string `json:"last_message_preview" gorm:"type:varchar(512);not null"`
	CreatedAt          int64  `json:"created_at" gorm:"not null;index"`
	UpdatedAt          int64  `json:"updated_at" gorm:"not null;index:idx_assistant_conversation_user_updated,priority:2"`
	ArchivedAt         int64  `json:"archived_at" gorm:"not null;default:0;index"`
}

func (AssistantConversation) TableName() string { return "assistant_conversations" }

// AssistantHistoryMessage is deliberately limited to text that has passed
// redactAssistantHistoryContent.  Secure values are represented by a card row
// rather than this content field.
type AssistantHistoryMessage struct {
	Id             int64  `json:"id" gorm:"primaryKey"`
	ConversationId int64  `json:"conversation_id" gorm:"not null;index:idx_assistant_history_conversation_id,priority:1"`
	Sequence       int    `json:"sequence" gorm:"not null;index:idx_assistant_history_conversation_id,priority:2"`
	Role           string `json:"role" gorm:"type:varchar(20);not null"`
	Content        string `json:"content" gorm:"type:text;not null"`
	CreatedAt      int64  `json:"created_at" gorm:"not null;index"`
}

func (AssistantHistoryMessage) TableName() string { return "assistant_history_messages" }

// AssistantSecureCard stores a short-lived encrypted value that must never be
// serialized through the conversation API.  The card can be revealed once by
// its owner through an authenticated browser action.
type AssistantSecureCard struct {
	Id             string `json:"-" gorm:"type:varchar(64);primaryKey"`
	OwnerUserId    int    `json:"-" gorm:"not null;index"`
	ConversationId int64  `json:"-" gorm:"not null;default:0;index"`
	MessageId      int64  `json:"-" gorm:"not null;default:0;index"`
	Type           string `json:"-" gorm:"type:varchar(32);not null"`
	Summary        string `json:"-" gorm:"type:varchar(255);not null"`
	Ciphertext     string `json:"-" gorm:"type:text;not null"`
	CreatedAt      int64  `json:"-" gorm:"not null;index"`
	ExpiresAt      int64  `json:"-" gorm:"not null;index"`
	RevealedAt     int64  `json:"-" gorm:"not null;default:0;index"`
}

func (AssistantSecureCard) TableName() string { return "assistant_secure_cards" }

type AssistantSecureCardView struct {
	// ID is intentionally empty for viewers other than the card owner.  An
	// opaque card identifier is still a bearer-like capability because it is
	// accepted by the reveal endpoint.
	ID     string `json:"id,omitempty"`
	Type   string `json:"type,omitempty"`
	Label  string `json:"label,omitempty"`
	Owner  string `json:"owner"`
	Shield bool   `json:"shield"`
}

type AssistantConversationView struct {
	Id                 int64  `json:"id"`
	Title              string `json:"title"`
	LastMessagePreview string `json:"last_message_preview"`
	CreatedAt          int64  `json:"created_at"`
	UpdatedAt          int64  `json:"updated_at"`
	ArchivedAt         int64  `json:"archived_at"`
	Owner              string `json:"owner"`
	PrivacyNotice      string `json:"privacy_notice"`
}

type AssistantHistoryMessageView struct {
	Id            int64                     `json:"id"`
	Role          string                    `json:"role"`
	Content       string                    `json:"content,omitempty"`
	Cards         []AssistantSecureCardView `json:"cards,omitempty"`
	CreatedAt     int64                     `json:"created_at"`
	PrivacyNotice string                    `json:"privacy_notice"`
}

// RedactAssistantHistoryContent is intentionally applied before a message is
// sent to the model and before it reaches persistent storage.  It favours
// false positives over retaining credentials in a support transcript.
func RedactAssistantHistoryContent(value string) string {
	value = strings.TrimSpace(value)
	value = assistantHistoryPEMPrivateKey.ReplaceAllString(value, "[REDACTED_PRIVATE_KEY]")
	value = assistantHistoryURLSecret.ReplaceAllString(value, "$1[REDACTED]")
	value = assistantHistoryCookiePattern.ReplaceAllString(value, "$1: [REDACTED]")
	value = assistantHistoryBearerPattern.ReplaceAllString(value, "Bearer [REDACTED_TOKEN]")
	value = assistantHistorySecretPattern.ReplaceAllString(value, "$1: [REDACTED]")
	value = assistantHistoryAPIKeyPattern.ReplaceAllString(value, "[REDACTED_API_KEY]")
	value = assistantHistoryJWTPattern.ReplaceAllString(value, "[REDACTED_TOKEN]")
	value = assistantHistoryEmailPattern.ReplaceAllString(value, "[REDACTED_EMAIL]")
	value = redactAssistantPhoneNumbers(value)
	value = redactAssistantIPAddresses(value)
	value = redactAssistantCardNumbers(value)
	return value
}

func redactAssistantPhoneNumbers(value string) string {
	return assistantHistoryPhonePattern.ReplaceAllString(value, "$1[REDACTED_PHONE]$3")
}

func redactAssistantIPAddresses(value string) string {
	value = assistantHistoryIPv4Pattern.ReplaceAllStringFunc(value, func(candidate string) string {
		if net.ParseIP(candidate) != nil {
			return "[REDACTED_IP]"
		}
		return candidate
	})
	return assistantHistoryIPv6Pattern.ReplaceAllStringFunc(value, func(candidate string) string {
		if !strings.Contains(candidate, ":") || net.ParseIP(candidate) == nil {
			return candidate
		}
		return "[REDACTED_IP]"
	})
}

func redactAssistantCardNumbers(value string) string {
	return assistantHistoryCardPattern.ReplaceAllStringFunc(value, func(candidate string) string {
		matches := assistantHistoryCardPattern.FindStringSubmatch(candidate)
		if len(matches) != 4 {
			return candidate
		}
		digits := strings.Map(func(r rune) rune {
			if r >= '0' && r <= '9' {
				return r
			}
			return -1
		}, matches[2])
		if len(digits) < 13 || len(digits) > 19 || !assistantLuhnValid(digits) {
			return candidate
		}
		return matches[1] + "[REDACTED_CARD]" + matches[3]
	})
}

func assistantLuhnValid(digits string) bool {
	sum := 0
	double := false
	for index := len(digits) - 1; index >= 0; index-- {
		digit := int(digits[index] - '0')
		if double {
			digit *= 2
			if digit > 9 {
				digit -= 9
			}
		}
		sum += digit
		double = !double
	}
	return sum%10 == 0
}

func redactAssistantHistoryBounded(value string) string {
	value = RedactAssistantHistoryContent(value)
	runes := []rune(value)
	if len(runes) > assistantHistoryMessageMaxRunes {
		return string(runes[:assistantHistoryMessageMaxRunes])
	}
	return value
}

func assistantConversationTitle(value string) string {
	value = redactAssistantHistoryBounded(value)
	runes := []rune(value)
	if len(runes) > assistantConversationTitleMaxRunes {
		return string(runes[:assistantConversationTitleMaxRunes])
	}
	return value
}

func assistantConversationRank(user *User) (int, error) {
	if user == nil || user.Id <= 0 {
		return 0, gorm.ErrInvalidData
	}
	if user.Role >= common.RoleRootUser {
		return 10_000, nil
	}
	if user.Role >= common.RoleAdminUser {
		return 1_000 + user.Role, nil
	}
	return 0, nil
}

// AuthorizeAssistantHistoryViewer implements the strict visibility lattice:
// a user always sees their own conversation; other conversations are visible
// only to an administrator with a strictly higher role. Ordinary account trust
// levels never grant access to another user's transcripts.
func AuthorizeAssistantHistoryViewer(viewerUserID, ownerUserID int) error {
	if viewerUserID <= 0 || ownerUserID <= 0 {
		return ErrAssistantHistoryForbidden
	}
	if viewerUserID == ownerUserID {
		return nil
	}
	var users []User
	if err := DB.Where("id IN ?", []int{viewerUserID, ownerUserID}).Find(&users).Error; err != nil {
		return err
	}
	var viewer, owner *User
	for index := range users {
		if users[index].Id == viewerUserID {
			viewer = &users[index]
		}
		if users[index].Id == ownerUserID {
			owner = &users[index]
		}
	}
	if viewer == nil || owner == nil {
		return ErrAssistantConversationNotFound
	}
	viewerRank, err := assistantConversationRank(viewer)
	if err != nil {
		return err
	}
	ownerRank, err := assistantConversationRank(owner)
	if err != nil {
		return err
	}
	if viewerRank <= ownerRank {
		return ErrAssistantHistoryForbidden
	}
	return nil
}

// PrepareAssistantConversation resolves a conversation for a new chat turn.
// Unlike reads, a user may continue only their own conversation.
func PrepareAssistantConversation(userID int, conversationID int64, firstMessage string) (*AssistantConversation, error) {
	if userID <= 0 {
		return nil, gorm.ErrInvalidData
	}
	if conversationID > 0 {
		var conversation AssistantConversation
		err := DB.Where("id = ? AND user_id = ?", conversationID, userID).First(&conversation).Error
		if errors.Is(err, gorm.ErrRecordNotFound) {
			return nil, ErrAssistantConversationNotFound
		}
		if err != nil {
			return nil, err
		}
		return &conversation, nil
	}
	now := common.GetTimestamp()
	conversation := AssistantConversation{
		UserId:             userID,
		Title:              assistantConversationTitle(firstMessage),
		LastMessagePreview: assistantConversationTitle(firstMessage),
		CreatedAt:          now,
		UpdatedAt:          now,
	}
	if err := DB.Create(&conversation).Error; err != nil {
		return nil, err
	}
	return &conversation, nil
}

func LoadAssistantConversationMessages(userID int, conversationID int64, limit int) ([]AssistantHistoryMessage, error) {
	conversation, err := PrepareAssistantConversation(userID, conversationID, "")
	if err != nil {
		return nil, err
	}
	if limit <= 0 || limit > assistantHistoryPageMax {
		limit = assistantHistoryPageMax
	}
	var messages []AssistantHistoryMessage
	if err := DB.Where("conversation_id = ?", conversation.Id).
		Where("role IN ?", []string{AssistantHistoryRoleUser, AssistantHistoryRoleAssistant}).
		Order("sequence ASC").Limit(limit).Find(&messages).Error; err != nil {
		return nil, err
	}
	return messages, nil
}

func appendAssistantHistoryMessageTx(tx *gorm.DB, conversationID int64, role, content string) (*AssistantHistoryMessage, error) {
	if role != AssistantHistoryRoleUser && role != AssistantHistoryRoleAssistant && role != AssistantHistoryRoleCard {
		return nil, gorm.ErrInvalidData
	}
	content = redactAssistantHistoryBounded(content)
	if role != AssistantHistoryRoleCard && strings.TrimSpace(content) == "" {
		return nil, gorm.ErrInvalidData
	}
	var last AssistantHistoryMessage
	if err := lockForUpdate(tx).Where("conversation_id = ?", conversationID).Order("sequence DESC").First(&last).Error; err != nil && !errors.Is(err, gorm.ErrRecordNotFound) {
		return nil, err
	}
	message := &AssistantHistoryMessage{
		ConversationId: conversationID,
		Sequence:       last.Sequence + 1,
		Role:           role,
		Content:        content,
		CreatedAt:      common.GetTimestamp(),
	}
	if err := tx.Create(message).Error; err != nil {
		return nil, err
	}
	return message, nil
}

// RecordAssistantConversationTurn records the new user turn and the final
// assistant text together.  Tool call payloads and upstream raw JSON are never
// persisted.
func RecordAssistantConversationTurn(userID int, conversationID int64, userContent, assistantContent string) error {
	if userID <= 0 || conversationID <= 0 {
		return gorm.ErrInvalidData
	}
	return DB.Transaction(func(tx *gorm.DB) error {
		var conversation AssistantConversation
		if err := lockForUpdate(tx).Where("id = ? AND user_id = ?", conversationID, userID).First(&conversation).Error; err != nil {
			if errors.Is(err, gorm.ErrRecordNotFound) {
				return ErrAssistantConversationNotFound
			}
			return err
		}
		if _, err := appendAssistantHistoryMessageTx(tx, conversationID, AssistantHistoryRoleUser, userContent); err != nil {
			return err
		}
		if _, err := appendAssistantHistoryMessageTx(tx, conversationID, AssistantHistoryRoleAssistant, assistantContent); err != nil {
			return err
		}
		now := common.GetTimestamp()
		return tx.Model(&conversation).Updates(map[string]any{
			"last_message_preview": assistantConversationTitle(assistantContent),
			"updated_at":           now,
		}).Error
	})
}

func setAssistantConversationArchived(userID int, conversationID int64, archived bool) (*AssistantConversation, error) {
	if userID <= 0 || conversationID <= 0 {
		return nil, ErrAssistantConversationNotFound
	}

	var updated AssistantConversation
	err := DB.Transaction(func(tx *gorm.DB) error {
		var conversation AssistantConversation
		if err := lockForUpdate(tx).
			Where("id = ? AND user_id = ?", conversationID, userID).
			First(&conversation).Error; err != nil {
			if errors.Is(err, gorm.ErrRecordNotFound) {
				return ErrAssistantConversationNotFound
			}
			return err
		}
		if archived {
			if conversation.ArchivedAt != 0 {
				return ErrAssistantConversationAlreadyArchived
			}
			conversation.ArchivedAt = common.GetTimestamp()
		} else {
			if conversation.ArchivedAt == 0 {
				return ErrAssistantConversationNotArchived
			}
			conversation.ArchivedAt = 0
		}
		if err := tx.Model(&conversation).Update("archived_at", conversation.ArchivedAt).Error; err != nil {
			return err
		}
		updated = conversation
		return nil
	})
	if err != nil {
		return nil, err
	}
	return &updated, nil
}

// ArchiveAssistantConversation changes only the owner's soft archive state.
// Visibility grants never confer mutation rights.
func ArchiveAssistantConversation(userID int, conversationID int64) (*AssistantConversation, error) {
	return setAssistantConversationArchived(userID, conversationID, true)
}

// UnarchiveAssistantConversation restores only the owner's soft archive state.
func UnarchiveAssistantConversation(userID int, conversationID int64) (*AssistantConversation, error) {
	return setAssistantConversationArchived(userID, conversationID, false)
}

func ListAssistantConversations(viewerUserID, ownerUserID int, limit int, archived bool) ([]AssistantConversationView, error) {
	if err := AuthorizeAssistantHistoryViewer(viewerUserID, ownerUserID); err != nil {
		return nil, err
	}
	if limit <= 0 || limit > assistantHistoryPageMax {
		limit = 30
	}
	var conversations []AssistantConversation
	archiveFilter := "archived_at = 0"
	if archived {
		archiveFilter = "archived_at <> 0"
	}
	if err := DB.Where("user_id = ?", ownerUserID).
		Where(archiveFilter).
		Order("updated_at DESC, id DESC").Limit(limit).Find(&conversations).Error; err != nil {
		return nil, err
	}
	owner := "lower_level_user"
	if viewerUserID == ownerUserID {
		owner = "self"
	}
	views := make([]AssistantConversationView, 0, len(conversations))
	for _, conversation := range conversations {
		views = append(views, AssistantConversationView{
			Id:                 conversation.Id,
			Title:              conversation.Title,
			LastMessagePreview: conversation.LastMessagePreview,
			CreatedAt:          conversation.CreatedAt,
			UpdatedAt:          conversation.UpdatedAt,
			ArchivedAt:         conversation.ArchivedAt,
			Owner:              owner,
			PrivacyNotice:      AssistantHistoryPrivacyNotice,
		})
	}
	return views, nil
}

func GetAssistantConversationHistory(viewerUserID int, conversationID int64, limit int) (*AssistantConversationView, []AssistantHistoryMessageView, error) {
	if conversationID <= 0 {
		return nil, nil, ErrAssistantConversationNotFound
	}
	var conversation AssistantConversation
	if err := DB.First(&conversation, conversationID).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			return nil, nil, ErrAssistantConversationNotFound
		}
		return nil, nil, err
	}
	if err := AuthorizeAssistantHistoryViewer(viewerUserID, conversation.UserId); err != nil {
		return nil, nil, err
	}
	if limit <= 0 || limit > assistantHistoryPageMax {
		limit = assistantHistoryPageMax
	}
	var messages []AssistantHistoryMessage
	if err := DB.Where("conversation_id = ?", conversation.Id).Order("sequence ASC").Limit(limit).Find(&messages).Error; err != nil {
		return nil, nil, err
	}
	var cards []AssistantSecureCard
	if err := DB.Where("conversation_id = ?", conversation.Id).Find(&cards).Error; err != nil {
		return nil, nil, err
	}
	cardsByMessageID := make(map[int64][]AssistantSecureCardView)
	standaloneCards := make([]AssistantSecureCardView, 0)
	for _, card := range cards {
		// Cards are intentionally shown as metadata to every authorized history
		// viewer.  Only the owner can call RevealAssistantSecureCard.
		cardView := assistantSecureCardView(card, viewerUserID == card.OwnerUserId)
		if card.MessageId > 0 {
			cardsByMessageID[card.MessageId] = append(cardsByMessageID[card.MessageId], cardView)
		} else {
			standaloneCards = append(standaloneCards, cardView)
		}
	}
	owner := "lower_level_user"
	if viewerUserID == conversation.UserId {
		owner = "self"
	}
	view := &AssistantConversationView{
		Id:                 conversation.Id,
		Title:              conversation.Title,
		LastMessagePreview: conversation.LastMessagePreview,
		CreatedAt:          conversation.CreatedAt,
		UpdatedAt:          conversation.UpdatedAt,
		ArchivedAt:         conversation.ArchivedAt,
		Owner:              owner,
		PrivacyNotice:      AssistantHistoryPrivacyNotice,
	}
	messageViews := make([]AssistantHistoryMessageView, 0, len(messages)+len(standaloneCards))
	for _, message := range messages {
		messageViews = append(messageViews, AssistantHistoryMessageView{
			Id:            message.Id,
			Role:          message.Role,
			Content:       message.Content,
			Cards:         cardsByMessageID[message.Id],
			CreatedAt:     message.CreatedAt,
			PrivacyNotice: AssistantHistoryPrivacyNotice,
		})
	}
	for _, card := range standaloneCards {
		messageViews = append(messageViews, AssistantHistoryMessageView{
			Role:          AssistantHistoryRoleCard,
			Cards:         []AssistantSecureCardView{card},
			PrivacyNotice: AssistantHistoryPrivacyNotice,
		})
	}
	return view, messageViews, nil
}

func secureCardEncryptionKey() [32]byte {
	return sha256.Sum256([]byte("assistant-secure-card-v1:" + common.SessionSecret))
}

func encryptAssistantSecureCardPayload(payload string) (string, error) {
	if len(payload) == 0 || len(payload) > assistantSecureCardPayloadMaxBytes {
		return "", gorm.ErrInvalidData
	}
	key := secureCardEncryptionKey()
	block, err := aes.NewCipher(key[:])
	if err != nil {
		return "", err
	}
	gcm, err := cipher.NewGCM(block)
	if err != nil {
		return "", err
	}
	nonce := make([]byte, gcm.NonceSize())
	if _, err := rand.Read(nonce); err != nil {
		return "", err
	}
	ciphertext := gcm.Seal(nil, nonce, []byte(payload), nil)
	return base64.RawURLEncoding.EncodeToString(append(nonce, ciphertext...)), nil
}

func decryptAssistantSecureCardPayload(ciphertext string) (string, error) {
	encoded, err := base64.RawURLEncoding.DecodeString(ciphertext)
	if err != nil {
		return "", err
	}
	key := secureCardEncryptionKey()
	block, err := aes.NewCipher(key[:])
	if err != nil {
		return "", err
	}
	gcm, err := cipher.NewGCM(block)
	if err != nil {
		return "", err
	}
	if len(encoded) < gcm.NonceSize() {
		return "", errors.New("secure card ciphertext is invalid")
	}
	plaintext, err := gcm.Open(nil, encoded[:gcm.NonceSize()], encoded[gcm.NonceSize():], nil)
	if err != nil {
		return "", err
	}
	return string(plaintext), nil
}

func newAssistantSecureCardID() (string, error) {
	random := make([]byte, 24)
	if _, err := rand.Read(random); err != nil {
		return "", err
	}
	return base64.RawURLEncoding.EncodeToString(random), nil
}

func CreateAssistantSecureCard(ownerUserID int, conversationID int64, cardType, summary, payload string) (*AssistantSecureCard, error) {
	if ownerUserID <= 0 || strings.TrimSpace(cardType) == "" || strings.TrimSpace(summary) == "" {
		return nil, gorm.ErrInvalidData
	}
	ciphertext, err := encryptAssistantSecureCardPayload(payload)
	if err != nil {
		return nil, err
	}
	id, err := newAssistantSecureCardID()
	if err != nil {
		return nil, err
	}
	now := common.GetTimestamp()
	card := &AssistantSecureCard{
		Id:             id,
		OwnerUserId:    ownerUserID,
		ConversationId: conversationID,
		Type:           cardType,
		Summary:        assistantConversationTitle(summary),
		Ciphertext:     ciphertext,
		CreatedAt:      now,
		ExpiresAt:      time.Now().Add(assistantSecureCardDefaultLifetime).Unix(),
	}
	if err := DB.Transaction(func(tx *gorm.DB) error {
		if conversationID > 0 {
			var conversation AssistantConversation
			if err := lockForUpdate(tx).Where("id = ? AND user_id = ?", conversationID, ownerUserID).First(&conversation).Error; err != nil {
				if errors.Is(err, gorm.ErrRecordNotFound) {
					return ErrAssistantConversationNotFound
				}
				return err
			}
			message, err := appendAssistantHistoryMessageTx(tx, conversationID, AssistantHistoryRoleCard, "")
			if err != nil {
				return err
			}
			card.MessageId = message.Id
		}
		return tx.Create(card).Error
	}); err != nil {
		return nil, err
	}
	return card, nil
}

// InsertAssistantTokenAndCreateSecureCard keeps credential creation atomic:
// either the user receives an opaque, owner-bound card or the token itself is
// not created.  This prevents an unusable API key from being left behind when
// secure-card storage is unavailable.
func InsertAssistantTokenAndCreateSecureCard(token *Token, ownerUserID int, conversationID int64, summary, payload string) (*AssistantSecureCard, error) {
	if token == nil || token.UserId <= 0 || token.UserId != ownerUserID {
		return nil, gorm.ErrInvalidData
	}
	ciphertext, err := encryptAssistantSecureCardPayload(payload)
	if err != nil {
		return nil, err
	}
	id, err := newAssistantSecureCardID()
	if err != nil {
		return nil, err
	}
	now := common.GetTimestamp()
	card := &AssistantSecureCard{
		Id:             id,
		OwnerUserId:    ownerUserID,
		ConversationId: conversationID,
		Type:           AssistantSecureCardTypeAPIKey,
		Summary:        assistantConversationTitle(summary),
		Ciphertext:     ciphertext,
		CreatedAt:      now,
		ExpiresAt:      time.Now().Add(assistantSecureCardDefaultLifetime).Unix(),
	}
	err = DB.Transaction(func(tx *gorm.DB) error {
		if conversationID > 0 {
			var conversation AssistantConversation
			if err := lockForUpdate(tx).Where("id = ? AND user_id = ?", conversationID, ownerUserID).First(&conversation).Error; err != nil {
				if errors.Is(err, gorm.ErrRecordNotFound) {
					return ErrAssistantConversationNotFound
				}
				return err
			}
			message, err := appendAssistantHistoryMessageTx(tx, conversationID, AssistantHistoryRoleCard, "")
			if err != nil {
				return err
			}
			card.MessageId = message.Id
		}
		if err := tx.Create(token).Error; err != nil {
			return err
		}
		if err := tx.Model(&User{}).
			Where("id = ? AND console_activated_at = ?", token.UserId, 0).
			Update("console_activated_at", time.Now().Unix()).Error; err != nil {
			return err
		}
		return tx.Create(card).Error
	})
	if err != nil {
		return nil, err
	}
	if err := invalidateUserCache(token.UserId); err != nil {
		common.SysLog("failed to invalidate user cache after assistant key creation: " + err.Error())
	}
	return card, nil
}

func assistantSecureCardView(card AssistantSecureCard, isOwner bool) AssistantSecureCardView {
	if !isOwner {
		return AssistantSecureCardView{
			Type:   "protected",
			Label:  "个人凭证已保护；仅所有者可查看",
			Owner:  "protected",
			Shield: true,
		}
	}
	return AssistantSecureCardView{
		ID:     card.Id,
		Type:   card.Type,
		Label:  card.Summary,
		Owner:  "self",
		Shield: true,
	}
}

// AssistantSecureCardViewForOwner returns only the opaque card metadata.  It
// deliberately has no path to serialize Ciphertext or the protected value.
func AssistantSecureCardViewForOwner(card *AssistantSecureCard) AssistantSecureCardView {
	if card == nil {
		return AssistantSecureCardView{}
	}
	return assistantSecureCardView(*card, true)
}

func RevealAssistantSecureCard(ownerUserID int, cardID string) (string, AssistantSecureCardView, error) {
	if ownerUserID <= 0 || strings.TrimSpace(cardID) == "" {
		return "", AssistantSecureCardView{}, gorm.ErrInvalidData
	}
	var payload string
	var view AssistantSecureCardView
	err := DB.Transaction(func(tx *gorm.DB) error {
		var card AssistantSecureCard
		if err := lockForUpdate(tx).Where("id = ? AND owner_user_id = ?", cardID, ownerUserID).First(&card).Error; err != nil {
			if errors.Is(err, gorm.ErrRecordNotFound) {
				return ErrAssistantSecureCardNotFound
			}
			return err
		}
		if card.RevealedAt > 0 {
			return ErrAssistantSecureCardConsumed
		}
		if card.ExpiresAt <= time.Now().Unix() {
			return ErrAssistantSecureCardExpired
		}
		plaintext, err := decryptAssistantSecureCardPayload(card.Ciphertext)
		if err != nil {
			return fmt.Errorf("decrypt assistant secure card: %w", err)
		}
		now := common.GetTimestamp()
		if result := tx.Model(&card).Where("revealed_at = ?", 0).Update("revealed_at", now); result.Error != nil {
			return result.Error
		} else if result.RowsAffected != 1 {
			return ErrAssistantSecureCardConsumed
		}
		payload = plaintext
		view = assistantSecureCardView(card, true)
		return nil
	})
	if err != nil {
		return "", AssistantSecureCardView{}, err
	}
	return payload, view, nil
}

func AssistantSecureCardPayload(value string) (map[string]string, error) {
	var payload map[string]string
	if err := json.Unmarshal([]byte(value), &payload); err != nil {
		return nil, err
	}
	return payload, nil
}
