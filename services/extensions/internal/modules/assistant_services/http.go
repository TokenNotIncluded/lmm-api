package assistant_services

import (
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"net/http"
	"strconv"
	"strings"
)

// ServeHTTP is mounted behind the existing credential bridge. Applying a
// draft cannot approve it; only Core's browser page can create human proof.
func (s *Service) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	credential, err := access.Credential(r)
	if err != nil {
		access.Respond(w, nil, err)
		return
	}
	parts := strings.Split(strings.Trim(strings.TrimPrefix(r.URL.Path, "/api/assistant/services/"), "/"), "/")
	if len(parts) >= 2 && parts[0] == "confirmations" && validSHA(parts[1]) {
		if len(parts) == 2 && r.Method == http.MethodGet {
			out, err := s.Confirmation(r.Context(), credential, parts[1])
			access.Respond(w, out, err)
			return
		}
		if len(parts) == 3 && parts[2] == "apply" && r.Method == http.MethodPost {
			var input struct{}
			if access.Body(w, r, &input) != nil {
				access.Respond(w, nil, access.ErrInvalid)
				return
			}
			out, err := s.ConfirmAfterProof(r.Context(), credential, parts[1])
			access.Respond(w, out, err)
			return
		}
	}
	if len(parts) == 2 && parts[0] == "support" && parts[1] == "queue" && r.Method == http.MethodGet {
		before, err := optionalCursor(r, "before")
		if err != nil {
			access.Respond(w, nil, err)
			return
		}
		out, err := s.SupportQueue(r.Context(), credential, before)
		access.Respond(w, map[string]any{"requests": out}, err)
		return
	}
	if len(parts) == 3 && parts[0] == "support" {
		id, err := strconv.ParseInt(parts[1], 10, 64)
		if err != nil || id <= 0 {
			access.Respond(w, nil, access.ErrInvalid)
			return
		}
		if parts[2] == "messages" {
			if r.Method == http.MethodGet {
				after, err := optionalCursor(r, "after")
				if err != nil {
					access.Respond(w, nil, err)
					return
				}
				out, err := s.SupportMessages(r.Context(), credential, id, after)
				access.Respond(w, map[string]any{"messages": out}, err)
				return
			}
			if r.Method == http.MethodPost {
				var input struct {
					ClientTurnID string `json:"client_turn_id"`
					Content      string `json:"content"`
				}
				if access.Body(w, r, &input) != nil {
					access.Respond(w, nil, access.ErrInvalid)
					return
				}
				out, err := s.SendSupportMessage(r.Context(), credential, id, input.ClientTurnID, input.Content)
				access.Respond(w, out, err)
				return
			}
		}
		if r.Method == http.MethodPost {
			switch parts[2] {
			case "accept":
				var input struct{}
				if access.Body(w, r, &input) != nil {
					access.Respond(w, nil, access.ErrInvalid)
					return
				}
				out, err := s.AcceptSupport(r.Context(), credential, id)
				access.Respond(w, out, err)
				return
			case "close":
				var input struct {
					Cancel bool `json:"cancel"`
				}
				if access.Body(w, r, &input) != nil {
					access.Respond(w, nil, access.ErrInvalid)
					return
				}
				out, err := s.CloseSupport(r.Context(), credential, id, input.Cancel)
				access.Respond(w, out, err)
				return
			}
		}
	}
	http.NotFound(w, r)
}
func optionalCursor(r *http.Request, key string) (int64, error) {
	values := r.URL.Query()[key]
	if len(values) == 0 {
		return 0, nil
	}
	if len(values) != 1 {
		return 0, access.ErrInvalid
	}
	value, err := strconv.ParseInt(values[0], 10, 64)
	if err != nil || value < 0 {
		return 0, access.ErrInvalid
	}
	return value, nil
}
