package assistant_services

import (
	"context"
	"encoding/json"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

type policyFixture map[string]PublishedPolicy

func (f policyFixture) PublishedPolicy(_ context.Context, _ string, document, language string) (PublishedPolicy, error) {
	v := f[document]
	v.Document = document
	v.Language = language
	if v.SourceLanguage == "" {
		v.SourceLanguage = language
	}
	v.Revision = strings.Repeat("a", 64)
	return v, nil
}
func policyCall(t *testing.T, s *Service, name, args string, level int32) (assistant.AgentToolResult, error) {
	t.Helper()
	return s.policyTool(context.Background(), assistant.AgentInvocation{Call: assistant.AgentToolCall{OperationID: "policy-operation-00001", Arguments: json.RawMessage(args), Scope: assistant.AgentToolScope{Name: name}}}, access.Principal{Level: level})
}
func policyJSON(t *testing.T, r assistant.AgentToolResult) map[string]any {
	t.Helper()
	var v map[string]any
	if json.Unmarshal(r.ResultJSON, &v) != nil {
		t.Fatal("invalid result")
	}
	return v
}
func TestPolicyUnicodePageAndFallback(t *testing.T) {
	s := &Service{dependencies: Dependencies{Policies: policyFixture{"user_agreement": {Text: "你K好世界", SourceLanguage: "zh-CN"}}}}
	r, e := policyCall(t, s, "get_site_policy", `{"document":"user_agreement","language":"en","offset":1,"limit":2}`, 0)
	if e != nil {
		t.Fatal(e)
	}
	v := policyJSON(t, r)
	if v["content"] != "K好" || v["next_offset"] != float64(3) || v["language_fallback"] != true || v["configured"] != false || v["source_is_untrusted"] != true {
		t.Fatalf("wrong page: %v", v)
	}
}
func TestPolicyLiteralUnicodeSearchAndSkips(t *testing.T) {
	s := &Service{dependencies: Dependencies{Policies: policyFixture{"user_agreement": {Text: "你KΣςſ .*[你好 kσS", Configured: true}, "privacy_policy": {Text: "https://example.invalid/private", Configured: true}}}}
	r, e := policyCall(t, s, "search_site_policies", `{"query":"kσs"}`, 0)
	if e != nil {
		t.Fatal(e)
	}
	v := policyJSON(t, r)
	if v["total"] != float64(1) {
		t.Fatalf("wrong count %v", v)
	}
	matches := v["matches"].([]any)
	if matches[0].(map[string]any)["match_offset"] != float64(12) {
		t.Fatalf("wrong Unicode source offset %v", matches)
	}
	if len(v["skipped_documents"].([]any)) != 2 {
		t.Fatalf("missing skipped metadata %v", v)
	}
	r, e = policyCall(t, s, "search_site_policies", `{"query":".*["}`, 0)
	if e != nil || policyJSON(t, r)["total"] != float64(1) {
		t.Fatal("search interpreted literal as regexp", e)
	}
}
func TestPolicyBoundsAndRootChanges(t *testing.T) {
	s := &Service{dependencies: Dependencies{Policies: policyFixture{"user_agreement": {Text: "old", Configured: true}}}}
	for _, raw := range []string{`{"document":"user_agreement","limit":6001}`, `{"document":"user_agreement","offset":-1}`, `{"document":"user_agreement","language":"de"}`, `{"document":"user_agreement","query":"extra"}`} {
		if _, e := policyCall(t, s, "get_site_policy", raw, 0); e == nil {
			t.Fatalf("accepted invalid %s", raw)
		}
	}
	args := `{"document":"user_agreement","language":"zh-CN","revision":"` + strings.Repeat("a", 64) + `","content":"new"}`
	if _, e := policyCall(t, s, "prepare_admin_site_policy_change", args, 5); e != access.ErrForbidden {
		t.Fatalf("root gate %v", e)
	}
	if _, e := policyCall(t, s, "prepare_admin_site_policy_change", args, 6); e != access.ErrUnavailable {
		t.Fatalf("missing Core authority must fail closed: %v", e)
	}
	args = strings.Replace(args, strings.Repeat("a", 64), strings.Repeat("b", 64), 1)
	r, e := policyCall(t, s, "prepare_admin_site_policy_change", args, 6)
	if e != nil || policyJSON(t, r)["status"] != "policy_changed" {
		t.Fatal("stale revision accepted", e)
	}
}
func TestServiceCatalogHasExactTwentyTwoBindings(t *testing.T) {
	s := &Service{}
	bindings := s.Bindings()
	if len(bindings) != 22 {
		t.Fatalf("bindings %d", len(bindings))
	}
	seen := map[string]bool{}
	for _, b := range bindings {
		if seen[b.Name] {
			t.Fatal("duplicate", b.Name)
		}
		seen[b.Name] = true
		var schema map[string]any
		if json.Unmarshal(b.Parameters, &schema) != nil {
			t.Fatal("invalid schema", b.Name)
		}
		if required, exists := schema["required"]; exists && required == nil {
			t.Fatal("JSON Schema required must be array, never null", b.Name)
		}
		if b.Executor != s || b.Version != assistant.FixedMainToolVersion || b.PriceSHA256 != assistant.FreeAgentPriceSHA256() {
			t.Fatal("wrong binding", b.Name)
		}
	}
	if _, e := assistant.NewAgentRegistry(bindings); e != nil {
		t.Fatal(e)
	}
	bindings[0].Parameters[0] = '!'
	if !json.Valid(s.Bindings()[0].Parameters) {
		t.Fatal("schema leaked mutable storage")
	}
}
