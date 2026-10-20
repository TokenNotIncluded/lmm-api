package assistant_services

import "testing"

func TestAvailableBindingsDoNotAdvertiseMissingDependencies(t *testing.T) {
	service := &Service{}
	available := service.AvailableBindings()
	expected := map[string]bool{"recall_memory": true, "remember_memory": true, "remember_profile_skill": true, "forget_profile_skill": true, "get_site_issues": true, "show_chart": true, "show_statistics": true, "show_choices": true, "show_flowchart": true, "calculate_math": true}
	if len(available) != len(expected) {
		t.Fatalf("available count=%d", len(available))
	}
	for _, binding := range available {
		if !expected[binding.Name] {
			t.Fatalf("unavailable tool advertised: %s", binding.Name)
		}
		delete(expected, binding.Name)
	}
	if len(expected) != 0 {
		t.Fatalf("missing local tools: %v", expected)
	}
	if service.ConfirmationAvailable() {
		t.Fatal("missing Core proof port advertised")
	}
	if len(service.Bindings()) != 22 {
		t.Fatalf("full frozen manifest=%d", len(service.Bindings()))
	}
}
