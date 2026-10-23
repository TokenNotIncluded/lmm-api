// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/url"
	"strconv"
	"strings"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

type AssistantHistory interface {
	Status(context.Context, string) (map[string]any, error)
	List(context.Context, string, int64, bool, int, string) (assistant.HistoryPage, error)
	Detail(context.Context, string, int64, int) (assistant.HistoryDetail, error)
	Archive(context.Context, string, int64, bool) (assistant.ArchiveResult, error)
	PresetClick(context.Context, string) error
}

type AssistantGreeting interface {
	Greeting(context.Context, string) (assistant.GreetingPreference, error)
	UpdateGreeting(context.Context, string, string, string, int64) (assistant.GreetingPreference, error)
}

type AssistantChat interface {
	Chat(context.Context, string, assistant.PersonalChatRequest) (assistant.PersonalChatReply, error)
	Recover(context.Context, string, int64, string) (assistant.PersonalChatReply, error)
}
type AssistantResume interface {
	Resume(context.Context, string, int64, string) (assistant.PersonalChatReply, error)
}

// RegisterAssistant preserves the old public presets, personal greeting and
// interactive history routes. A shared host must wrap these routes in its own
// admission/drain gate.
// No browser may inject assistant messages or select a payer/model/owner.
func RegisterAssistant(mux *http.ServeMux, auth *Authenticator, history AssistantHistory, chatPorts ...AssistantChat) {
	var chat AssistantChat
	if len(chatPorts) == 1 && !access.Missing(chatPorts[0]) {
		chat = chatPorts[0]
	}
	gifts, _ := chat.(AssistantGifts)
	protected := func(fn func(http.ResponseWriter, *http.Request, string)) http.HandlerFunc {
		return func(w http.ResponseWriter, r *http.Request) {
			if _, err := auth.Verify(r.Context(), r); err != nil {
				assistantRespond(w, nil, assistantAuthError(err))
				return
			}
			if access.Missing(history) {
				assistantRespond(w, nil, access.ErrUnavailable)
				return
			}
			token, _ := strings.CutPrefix(r.Header.Get("Authorization"), "Bearer ")
			fn(w, r, token)
		}
	}
	public := func(fn func(http.ResponseWriter, *http.Request)) http.HandlerFunc {
		return func(w http.ResponseWriter, r *http.Request) {
			for name := range r.Header {
				if strings.EqualFold(name, "Authorization") {
					if _, err := auth.Verify(r.Context(), r); err != nil {
						assistantRespond(w, nil, assistantAuthError(err))
						return
					}
					break
				}
			}
			if access.Missing(history) {
				assistantRespond(w, nil, access.ErrUnavailable)
				return
			}
			fn(w, r)
		}
	}
	mux.HandleFunc("GET /api/assistant/pre-conversation-presets", public(func(w http.ResponseWriter, r *http.Request) {
		query, err := assistantQuery(r)
		if err != nil {
			assistantRespond(w, nil, err)
			return
		}
		assistantRespond(w, assistant.HistoryPresets(query.Get("language")), nil)
	}))
	mux.HandleFunc("POST /api/assistant/pre-conversation-presets/{id}/click", public(func(w http.ResponseWriter, r *http.Request) {
		assistantRespond(w, nil, history.PresetClick(r.Context(), r.PathValue("id")))
	}))
	greeting, greetingAvailable := history.(AssistantGreeting)
	mux.HandleFunc("GET /api/assistant/workspace/greeting", protected(func(w http.ResponseWriter, r *http.Request, token string) {
		if !greetingAvailable || access.Missing(greeting) {
			assistantRespond(w, nil, access.ErrUnavailable)
			return
		}
		query, err := assistantQuery(r)
		if err != nil || len(query) != 0 {
			assistantRespond(w, nil, access.ErrInvalid)
			return
		}
		value, err := greeting.Greeting(r.Context(), token)
		assistantRespond(w, value, err)
	}))
	mux.HandleFunc("PUT /api/assistant/workspace/greeting", protected(func(w http.ResponseWriter, r *http.Request, token string) {
		if !greetingAvailable || access.Missing(greeting) {
			assistantRespond(w, nil, access.ErrUnavailable)
			return
		}
		query, err := assistantQuery(r)
		if err != nil || len(query) != 0 {
			assistantRespond(w, nil, access.ErrInvalid)
			return
		}
		var body struct {
			Language *string `json:"language"`
			Template *string `json:"template"`
			Revision *int64  `json:"revision"`
		}
		if err = access.Body(w, r, &body); err != nil || body.Language == nil || body.Template == nil || body.Revision == nil {
			assistantRespond(w, nil, access.ErrInvalid)
			return
		}
		value, err := greeting.UpdateGreeting(r.Context(), token, *body.Language, *body.Template, *body.Revision)
		assistantRespond(w, value, err)
	}))
	mux.HandleFunc("GET /api/assistant/status", protected(func(w http.ResponseWriter, r *http.Request, token string) {
		value, err := history.Status(r.Context(), token)
		if err == nil && !access.Missing(chat) {
			value["enabled"], value["route_available"] = true, true
			value["funding"] = map[string]any{"mode": "personal", "configured": true}
			value["agent"] = map[string]any{"enabled": true, "stream_enabled": false}
			value["unavailable_reason"] = ""
		}
		if err == nil && !access.Missing(gifts) && gifts.GiftsAvailable() {
			if capabilities, ok := value["capabilities"].(map[string]bool); ok {
				capabilities["new_user_gift"] = true
			}
		}
		if agent, ok := chat.(interface{ MultiRoundAvailable() bool }); err == nil && ok && agent.MultiRoundAvailable() {
			value["agent"] = map[string]any{"enabled": true, "stream_enabled": false, "multi_round": true, "explicit_resume": true}
			value["funding"] = map[string]any{"mode": "core_policy", "configured": true}
		}
		assistantRespond(w, value, err)
	}))
	mux.HandleFunc("GET /api/assistant/conversations", protected(func(w http.ResponseWriter, r *http.Request, token string) {
		query, err := assistantQuery(r)
		if err != nil {
			assistantRespond(w, nil, err)
			return
		}
		limit, err := assistantLimit(query, 30)
		var owner int64
		var archived bool
		if raw, ok := query["user_id"]; ok && err == nil {
			owner, err = strconv.ParseInt(raw[0], 10, 64)
			if owner <= 0 {
				err = access.ErrInvalid
			}
		}
		if raw, ok := query["archived"]; ok && err == nil {
			archived, err = strconv.ParseBool(raw[0])
		}
		if err != nil {
			assistantRespond(w, nil, access.ErrInvalid)
			return
		}
		value, err := history.List(r.Context(), token, owner, archived, limit, query.Get("cursor"))
		assistantRespond(w, value, err)
	}))
	mux.HandleFunc("GET /api/assistant/conversations/{id}", protected(func(w http.ResponseWriter, r *http.Request, token string) {
		id, err := assistantConversationID(r)
		if err != nil {
			assistantRespond(w, nil, err)
			return
		}
		query, err := assistantQuery(r)
		if err != nil {
			assistantRespond(w, nil, err)
			return
		}
		limit, err := assistantLimit(query, 100)
		if err != nil {
			assistantRespond(w, nil, err)
			return
		}
		value, err := history.Detail(r.Context(), token, id, limit)
		assistantRespond(w, value, err)
	}))
	for _, operation := range []string{"archive", "unarchive"} {
		archived := operation == "archive"
		mux.HandleFunc("POST /api/assistant/conversations/{id}/"+operation, protected(func(w http.ResponseWriter, r *http.Request, token string) {
			id, err := assistantConversationID(r)
			if err != nil {
				assistantRespond(w, nil, err)
				return
			}
			var body struct{}
			if err = access.Body(w, r, &body); err != nil {
				assistantRespond(w, nil, err)
				return
			}
			value, err := history.Archive(r.Context(), token, id, archived)
			assistantRespond(w, value, err)
		}))
	}
	RegisterAssistantGifts(mux, auth, gifts)
	mux.HandleFunc("POST /api/assistant/chat", protected(func(w http.ResponseWriter, r *http.Request, token string) {
		if access.Missing(chat) {
			assistantRespond(w, nil, access.ErrUnavailable)
			return
		}
		var body struct {
			ConversationID int64  `json:"conversation_id"`
			ClientTurnID   string `json:"client_turn_id"`
			Message        string `json:"message"`
			Messages       []struct {
				Role    string `json:"role"`
				Content string `json:"content"`
			} `json:"messages"`
			PresetID string `json:"preset_id"`
		}
		query, err := assistantQuery(r)
		if err != nil || len(query) != 0 || access.Body(w, r, &body) != nil {
			assistantRespond(w, nil, access.ErrInvalid)
			return
		}
		// Browser history is display context, never Core authority or a system
		// prompt. Core reconstructs its own conversation. Custom presets need a
		// server-owned policy contract before they can change model behavior.
		if body.PresetID != "" || len(body.Messages) > 64 {
			assistantRespond(w, nil, access.ErrInvalid)
			return
		}
		for _, message := range body.Messages {
			if (message.Role != "user" && message.Role != "assistant") || len(message.Content) > 32768 {
				assistantRespond(w, nil, access.ErrInvalid)
				return
			}
		}
		reply, err := chat.Chat(r.Context(), token, assistant.PersonalChatRequest{ConversationID: body.ConversationID, ClientTurnID: body.ClientTurnID, Message: body.Message})
		assistantChatRespond(w, reply, err)
	}))
	mux.HandleFunc("GET /api/assistant/conversations/{id}/turns/{turn}", protected(func(w http.ResponseWriter, r *http.Request, token string) {
		if access.Missing(chat) {
			assistantRespond(w, nil, access.ErrUnavailable)
			return
		}
		id, err := assistantConversationID(r)
		query, queryErr := assistantQuery(r)
		if err != nil || queryErr != nil || len(query) != 0 {
			assistantRespond(w, nil, access.ErrInvalid)
			return
		}
		reply, err := chat.Recover(r.Context(), token, id, r.PathValue("turn"))
		assistantChatRespond(w, reply, err)
	}))
	mux.HandleFunc("POST /api/assistant/conversations/{id}/turns/{turn}/resume", protected(func(w http.ResponseWriter, r *http.Request, token string) {
		resume, ok := chat.(AssistantResume)
		if !ok || access.Missing(resume) {
			assistantRespond(w, nil, access.ErrUnavailable)
			return
		}
		id, err := assistantConversationID(r)
		query, queryErr := assistantQuery(r)
		var body struct{}
		if err != nil || queryErr != nil || len(query) != 0 || access.Body(w, r, &body) != nil {
			assistantRespond(w, nil, access.ErrInvalid)
			return
		}
		reply, err := resume.Resume(r.Context(), token, id, r.PathValue("turn"))
		assistantChatRespond(w, reply, err)
	}))
}

func assistantChatRespond(w http.ResponseWriter, reply assistant.PersonalChatReply, err error) {
	if err != nil {
		assistantRespond(w, nil, err)
		return
	}
	if reply.State != "settled" || reply.Text == "" {
		w.Header().Set("Content-Type", "application/json; charset=utf-8")
		w.Header().Set("Cache-Control", "no-store")
		w.Header().Set("X-Content-Type-Options", "nosniff")
		status, code := http.StatusAccepted, "ASSISTANT_OPERATION_PENDING"
		if reply.State == "released" {
			status, code = http.StatusConflict, "ASSISTANT_OPERATION_RELEASED"
		}
		switch reply.State {
		case "rejected":
			status, code = http.StatusConflict, "ASSISTANT_OPERATION_REJECTED"
		case "expired":
			status, code = http.StatusConflict, "ASSISTANT_OPERATION_EXPIRED"
		case "human_support_pending":
			code = "ASSISTANT_HUMAN_SUPPORT_PENDING"
		}
		w.WriteHeader(status)
		_ = json.NewEncoder(w).Encode(map[string]any{"success": false, "code": code, "data": reply})
		return
	}
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	body := map[string]any{
		"success":               true,
		"choices":               []any{map[string]any{"message": map[string]string{"role": "assistant", "content": reply.Text}}},
		"lmm_assistant_history": map[string]any{"conversation_id": reply.ConversationID, "client_turn_id": reply.ClientTurnID},
	}
	if reply.Gift != nil {
		body["lmm_assistant_gift"] = reply.Gift
	}
	_ = json.NewEncoder(w).Encode(body)
}

func assistantConversationID(r *http.Request) (int64, error) {
	id, err := strconv.ParseInt(r.PathValue("id"), 10, 64)
	if err != nil || id <= 0 || id > 9007199254740991 {
		return 0, access.ErrNotFound
	}
	return id, nil
}
func assistantQuery(r *http.Request) (url.Values, error) {
	query, err := url.ParseQuery(r.URL.RawQuery)
	if err != nil || len(r.URL.RawQuery) > 2048 {
		return nil, access.ErrInvalid
	}
	for _, values := range query {
		if len(values) != 1 {
			return nil, access.ErrInvalid
		}
	}
	return query, nil
}
func assistantLimit(query url.Values, fallback int) (int, error) {
	if raw, ok := query["limit"]; ok {
		limit, err := strconv.Atoi(raw[0])
		if err != nil || limit < 1 || limit > 100 {
			return 0, access.ErrInvalid
		}
		return limit, nil
	}
	return fallback, nil
}
func assistantAuthError(err error) error {
	for _, pair := range [][2]error{{store.ErrUnauthorized, access.ErrUnauthorized}, {store.ErrForbidden, access.ErrForbidden}, {store.ErrInvalid, access.ErrInvalid}} {
		if errors.Is(err, pair[0]) {
			return pair[1]
		}
	}
	return access.ErrUnavailable
}

func assistantRespond(w http.ResponseWriter, value any, err error) {
	code := http.StatusOK
	body := map[string]any{"success": true, "message": "", "data": value}
	if err != nil {
		code = http.StatusServiceUnavailable
		label := "ASSISTANT_UNAVAILABLE"
		for _, pair := range []struct {
			err   error
			code  int
			label string
		}{{access.ErrInvalid, 400, "ASSISTANT_INVALID_REQUEST"}, {access.ErrUnauthorized, 401, "ASSISTANT_SESSION_REQUIRED"},
			{access.ErrForbidden, 403, "ASSISTANT_ACCESS_DENIED"}, {access.ErrNotFound, 404, "ASSISTANT_HISTORY_NOT_FOUND"},
			{access.ErrConflict, 409, "ASSISTANT_HISTORY_CONFLICT"}, {access.ErrLimit, 429, "ASSISTANT_LIMIT_REACHED"}} {
			if errors.Is(err, pair.err) {
				code, label = pair.code, pair.label
				break
			}
		}
		body = map[string]any{"success": false, "code": label, "message": "Assistant request could not be completed."}
	}
	data, marshalErr := json.Marshal(body)
	if marshalErr != nil {
		code = http.StatusServiceUnavailable
		data = []byte(`{"success":false,"code":"ASSISTANT_UNAVAILABLE","message":"Assistant request could not be completed."}`)
	}
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.WriteHeader(code)
	_, _ = w.Write(append(data, '\n'))
}
