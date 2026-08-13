package controller

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"strings"
	"sync"
	"time"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/pkg/cachex"
	"github.com/QuantumNous/new-api/setting"
	"github.com/samber/hot"
)

const assistantResponseCacheNamespace = "new-api:assistant-response:v2"

type assistantCachedResponse struct {
	Status            int    `json:"status"`
	Body              []byte `json:"body"`
	ConversationTitle string `json:"conversation_title,omitempty"`
}

var (
	assistantResponseCacheOnce sync.Once
	assistantResponseCache     *cachex.HybridCache[assistantCachedResponse]
	assistantCacheGates        = struct {
		sync.Mutex
		entries map[string]*assistantCacheGate
	}{entries: make(map[string]*assistantCacheGate)}
)

// assistantCacheGate prevents a burst of identical, cache-eligible questions
// from all reaching the upstream model before the first response is stored.
// The gate is deliberately narrower than the response cache: it never shares
// a tool result or a response between users, and callers re-check the cache
// after waiting for the current owner.
type assistantCacheGate struct {
	done chan struct{}
}

// acquireAssistantCacheGate serializes only the same cache key. The returned
// release function is idempotent so middleware error paths can safely defer it.
func acquireAssistantCacheGate(ctx context.Context, key string) (func(), bool) {
	if key == "" {
		return func() {}, true
	}
	for {
		assistantCacheGates.Lock()
		entry, exists := assistantCacheGates.entries[key]
		if !exists {
			entry = &assistantCacheGate{done: make(chan struct{})}
			assistantCacheGates.entries[key] = entry
			assistantCacheGates.Unlock()

			var once sync.Once
			return func() {
				once.Do(func() {
					assistantCacheGates.Lock()
					if current, ok := assistantCacheGates.entries[key]; ok && current == entry {
						delete(assistantCacheGates.entries, key)
						close(entry.done)
					}
					assistantCacheGates.Unlock()
				})
			}, true
		}
		wait := entry.done
		assistantCacheGates.Unlock()

		select {
		case <-wait:
			// The owner has finished. Re-check the map because another request
			// may have become the next owner before this waiter woke up.
		case <-ctx.Done():
			return func() {}, false
		}
	}
}

func getAssistantResponseCache() *cachex.HybridCache[assistantCachedResponse] {
	assistantResponseCacheOnce.Do(func() {
		assistantResponseCache = cachex.NewHybridCache[assistantCachedResponse](cachex.HybridCacheConfig[assistantCachedResponse]{
			Namespace: cachex.Namespace(assistantResponseCacheNamespace),
			Redis:     common.RDB,
			RedisEnabled: func() bool {
				return common.RedisEnabled && common.RDB != nil
			},
			RedisCodec: cachex.JSONCodec[assistantCachedResponse]{},
			Memory: func() *hot.HotCache[string, assistantCachedResponse] {
				return hot.NewHotCache[string, assistantCachedResponse](hot.LRU, 2048).
					WithTTL(7 * 24 * time.Hour).
					WithJanitor().
					Build()
			},
		})
	})
	return assistantResponseCache
}

type assistantCacheContext struct {
	UserID                   int                      `json:"user_id"`
	Username                 string                   `json:"username,omitempty"`
	Email                    string                   `json:"email,omitempty"`
	EmailDomain              string                   `json:"email_domain,omitempty"`
	EmailCategory            string                   `json:"email_category,omitempty"`
	AuthProviders            []string                 `json:"auth_providers,omitempty"`
	AccessLevel              string                   `json:"access_level"`
	DeveloperAccessGranted   bool                     `json:"developer_access_granted"`
	AccessReviewStatus       string                   `json:"access_review_status,omitempty"`
	PaymentMethodsHidden     bool                     `json:"payment_methods_hidden"`
	PaymentRestrictionCauses []string                 `json:"payment_restriction_causes,omitempty"`
	CustomerProfile          assistantCustomerProfile `json:"customer_profile"`
	Intent                   string                   `json:"current_intent,omitempty"`
	ProfileSignals           []string                 `json:"profile_signals,omitempty"`
}

func toAssistantCacheContext(context assistantUserContext) assistantCacheContext {
	return assistantCacheContext{
		UserID:                   context.UserID,
		Username:                 context.Username,
		Email:                    context.Email,
		EmailDomain:              context.EmailDomain,
		EmailCategory:            context.EmailCategory,
		AuthProviders:            append([]string(nil), context.AuthProviders...),
		AccessLevel:              context.AccessLevel,
		DeveloperAccessGranted:   context.DeveloperAccessGranted,
		AccessReviewStatus:       context.AccessReviewStatus,
		PaymentMethodsHidden:     context.PaymentMethodsHidden,
		PaymentRestrictionCauses: append([]string(nil), context.PaymentRestrictionCauses...),
		CustomerProfile:          context.CustomerProfile,
		Intent:                   context.Intent,
		ProfileSignals:           append([]string(nil), context.ProfileSignals...),
	}
}

func assistantCacheConversation(conversation []assistantOpenAIMessage) []assistantOpenAIMessage {
	result := make([]assistantOpenAIMessage, len(conversation))
	copy(result, conversation)
	for index := range result {
		result[index].Content = strings.ToLower(strings.Join(strings.Fields(result[index].Content), " "))
	}
	return result
}

func assistantCacheKey(settings setting.AssistantSettings, conversation []assistantOpenAIMessage, contexts ...assistantUserContext) string {
	if !settings.CacheEnabled || settings.CacheTTLMinutes <= 0 || len(conversation) != 1 || conversation[0].Role != "user" {
		return ""
	}
	var userContext assistantUserContext
	if len(contexts) > 0 {
		userContext = contexts[0]
	}
	// Title generation is metadata work for a new conversation. It must not
	// change the identity of the user's question, otherwise a replay after a
	// lost first response cannot hit the response that was already cached.
	cachePromptContext := userContext
	cachePromptContext.ConversationTitleNeeded = false

	fingerprint := struct {
		Version          string                   `json:"version"`
		Model            string                   `json:"model"`
		SystemPrompt     string                   `json:"system_prompt"`
		AgentLoopEnabled bool                     `json:"agent_loop_enabled"`
		MaxSteps         int                      `json:"max_steps"`
		TimeoutSeconds   int                      `json:"timeout_seconds"`
		TTLMinutes       int                      `json:"ttl_minutes"`
		UserContext      assistantCacheContext    `json:"user_context"`
		Conversation     []assistantOpenAIMessage `json:"conversation"`
	}{
		Version:          "assistant-cache-v2",
		Model:            settings.Model,
		SystemPrompt:     buildAssistantSystemPrompt(settings, cachePromptContext),
		AgentLoopEnabled: settings.AgentLoopEnabled,
		MaxSteps:         settings.MaxSteps,
		TimeoutSeconds:   settings.TimeoutSeconds,
		TTLMinutes:       settings.CacheTTLMinutes,
		UserContext:      toAssistantCacheContext(userContext),
		Conversation:     assistantCacheConversation(conversation),
	}
	raw, err := json.Marshal(fingerprint)
	if err != nil {
		return ""
	}
	digest := sha256.Sum256(raw)
	return hex.EncodeToString(digest[:])
}

func getAssistantCachedResponse(key string) (assistantCachedResponse, bool) {
	if key == "" {
		return assistantCachedResponse{}, false
	}
	value, found, err := getAssistantResponseCache().Get(key)
	if err != nil {
		common.SysLog("assistant response cache read failed: " + err.Error())
		return assistantCachedResponse{}, false
	}
	if !found || value.Status < 200 || value.Status >= 300 || len(value.Body) == 0 {
		return assistantCachedResponse{}, false
	}
	normalized, err := normalizeAssistantClientResponse(nil, value.Body)
	if err != nil {
		return assistantCachedResponse{}, false
	}
	value.Body = normalized
	return value, true
}

func storeAssistantCachedResponse(settings setting.AssistantSettings, key string, status int, body []byte, conversationTitles ...string) {
	if key == "" || !settings.CacheEnabled || settings.CacheTTLMinutes <= 0 || status < 200 || status >= 300 || len(body) == 0 {
		return
	}
	normalized, err := normalizeAssistantClientResponse(nil, body)
	if err != nil {
		return
	}
	conversationTitle := ""
	if len(conversationTitles) > 0 {
		conversationTitle = strings.TrimSpace(conversationTitles[0])
	}
	value := assistantCachedResponse{Status: status, Body: normalized, ConversationTitle: conversationTitle}
	if err := getAssistantResponseCache().SetWithTTL(key, value, time.Duration(settings.CacheTTLMinutes)*time.Minute); err != nil {
		common.SysLog("assistant response cache write failed: " + err.Error())
	}
}
