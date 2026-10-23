package assistant_services

import (
	"context"
	"database/sql"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

type ToolAction func(context.Context, *sql.Tx, access.Principal, assistant.AgentInvocation) (any, error)
type toolAction struct {
	minimumLevel int32
	apply        ToolAction
}

// Trusted composition only: snapshots code callbacks, never model definitions.
func (s *Service) WithToolAction(name string, minimumLevel int32, action ToolAction) (*Service, error) {
	if s == nil || name == "" || len(name) > 80 || minimumLevel < 0 || minimumLevel > 6 || action == nil {
		return nil, access.ErrInvalid
	}
	if _, ok := s.actions[name]; ok {
		return nil, access.ErrConflict
	}
	out := *s
	out.actions = make(map[string]toolAction, len(s.actions)+1)
	for key, value := range s.actions {
		out.actions[key] = value
	}
	out.actions[name] = toolAction{minimumLevel, action}
	return &out, nil
}
