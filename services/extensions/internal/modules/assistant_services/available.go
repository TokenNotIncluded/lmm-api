package assistant_services

import (
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

// AvailableBindings advertises only complete installed domain dependencies.
// Bindings remains the frozen full manifest for source/protocol comparison.
// Hosts must establish readiness of remote ports before supplying them to New.
func (s *Service) AvailableBindings() []assistant.AgentToolBinding {
	if s == nil {
		return nil
	}
	out := []assistant.AgentToolBinding{}
	for _, binding := range s.Bindings() {
		enabled := true
		switch binding.Name {
		case "search_web":
			enabled = !access.Missing(s.dependencies.Search)
		case "get_service_facts", "get_setup_guide":
			enabled = !access.Missing(s.dependencies.Public)
		case "navigate_to_page":
			enabled = !access.Missing(s.dependencies.Navigation)
		case "get_site_policy", "search_site_policies":
			enabled = !access.Missing(s.dependencies.Policies)
		case "prepare_admin_site_policy_change":
			enabled = !access.Missing(s.dependencies.Policies) && !access.Missing(s.dependencies.PolicyChanges)
		case "create_site_issue", "update_site_issue", "request_human_support":
			enabled = s.ConfirmationAvailable()
		case "get_human_support_status", "book_technical_support":
			enabled = !access.Missing(s.dependencies.SupportEligibility)
		}
		if enabled {
			out = append(out, binding)
		}
	}
	return out
}
func (s *Service) ConfirmationAvailable() bool {
	return s != nil && !access.Missing(s.dependencies.Confirmations)
}
