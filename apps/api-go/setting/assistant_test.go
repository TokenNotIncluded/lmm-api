package setting

import (
	"net"
	"strconv"
	"testing"
)

func TestAssistantDefaultsAndValidation(t *testing.T) {
	settings := GetAssistantSettings()
	if !settings.Enabled {
		t.Fatal("assistant should be enabled by default")
	}
	if settings.Model != DefaultAssistantModel {
		t.Fatalf("unexpected default model: %q", settings.Model)
	}
	if !settings.AgentLoopEnabled || settings.MaxSteps != 6 || settings.TimeoutSeconds != 45 || !settings.CacheEnabled || settings.CacheTTLMinutes != 1440 {
		t.Fatalf("unexpected assistant runtime defaults: %+v", settings)
	}
	if !settings.RetentionEnabled || settings.ActiveRetentionDays != 90 || settings.ArchivedRetentionDays != 30 || settings.SecurityRetentionDays != 180 || settings.RetentionIntervalHours != 24 {
		t.Fatalf("unexpected assistant retention defaults: %+v", settings)
	}

	invalid := map[string]string{
		AssistantModelOptionKey:                  " ",
		AssistantMaxStepsOptionKey:               "13",
		AssistantTimeoutSecondsOptionKey:         "4",
		AssistantCacheTTLMinutesOptionKey:        "10081",
		AssistantActiveRetentionDaysOptionKey:    "6",
		AssistantArchivedRetentionDaysOptionKey:  "0",
		AssistantSecurityRetentionDaysOptionKey:  "29",
		AssistantRetentionIntervalHoursOptionKey: "169",
	}
	for key, value := range invalid {
		if err := ValidateAssistantOption(key, value); err == nil {
			t.Fatalf("expected validation error for %s=%q", key, value)
		}
	}
	if err := ValidateAssistantOption(AssistantWeeklyCreditUSDOptionKey, "retired-value"); err != nil {
		t.Fatalf("retired weekly credit option must remain write-compatible: %v", err)
	}
}

func TestAssistantSettingsUpdates(t *testing.T) {
	original := GetAssistantSettings()
	t.Cleanup(func() {
		SetAssistantEnabled(original.Enabled)
		_ = UpdateAssistantModel(original.Model)
		SetAssistantAgentLoopEnabled(original.AgentLoopEnabled)
		_ = UpdateAssistantMaxSteps(strconv.Itoa(original.MaxSteps))
		_ = UpdateAssistantTimeoutSeconds(strconv.Itoa(original.TimeoutSeconds))
		SetAssistantCacheEnabled(original.CacheEnabled)
		_ = UpdateAssistantCacheTTLMinutes(strconv.Itoa(original.CacheTTLMinutes))
		SetAssistantRetentionEnabled(original.RetentionEnabled)
		_ = UpdateAssistantActiveRetentionDays(strconv.Itoa(original.ActiveRetentionDays))
		_ = UpdateAssistantArchivedRetentionDays(strconv.Itoa(original.ArchivedRetentionDays))
		_ = UpdateAssistantSecurityRetentionDays(strconv.Itoa(original.SecurityRetentionDays))
		_ = UpdateAssistantRetentionIntervalHours(strconv.Itoa(original.RetentionIntervalHours))
	})

	SetAssistantEnabled(false)
	if err := UpdateAssistantModel(" custom-model "); err != nil {
		t.Fatal(err)
	}
	SetAssistantAgentLoopEnabled(false)
	if err := UpdateAssistantMaxSteps("9"); err != nil {
		t.Fatal(err)
	}
	if err := UpdateAssistantTimeoutSeconds("60"); err != nil {
		t.Fatal(err)
	}
	SetAssistantCacheEnabled(false)
	if err := UpdateAssistantCacheTTLMinutes("30"); err != nil {
		t.Fatal(err)
	}
	SetAssistantRetentionEnabled(false)
	if err := UpdateAssistantActiveRetentionDays("120"); err != nil {
		t.Fatal(err)
	}
	if err := UpdateAssistantArchivedRetentionDays("45"); err != nil {
		t.Fatal(err)
	}
	if err := UpdateAssistantSecurityRetentionDays("365"); err != nil {
		t.Fatal(err)
	}
	if err := UpdateAssistantRetentionIntervalHours("12"); err != nil {
		t.Fatal(err)
	}

	settings := GetAssistantSettings()
	if settings.Enabled || settings.Model != "custom-model" || settings.AgentLoopEnabled || settings.MaxSteps != 9 || settings.TimeoutSeconds != 60 || settings.CacheEnabled || settings.CacheTTLMinutes != 30 || settings.RetentionEnabled || settings.ActiveRetentionDays != 120 || settings.ArchivedRetentionDays != 45 || settings.SecurityRetentionDays != 365 || settings.RetentionIntervalHours != 12 {
		t.Fatalf("unexpected updated settings: %+v", settings)
	}
}

func TestAssistantSearchURLValidationBlocksPrivateTargetsAndCredentials(t *testing.T) {
	valid := []string{
		"https://search.example.com/api/search?q=initial",
		"http://8.8.8.8/search",
	}
	for _, value := range valid {
		if err := ValidateAssistantSearchURL(value); err != nil {
			t.Fatalf("expected search URL %q to be valid: %v", value, err)
		}
	}
	invalid := []string{
		"ftp://search.example.com/api",
		"http://user:password@search.example.com/api",
		"http://127.0.0.1/api",
		"http://10.0.0.7/api",
		"http://169.254.169.254/latest/meta-data",
		"http://[::1]/api",
	}
	for _, value := range invalid {
		if err := ValidateAssistantSearchURL(value); err == nil {
			t.Fatalf("expected search URL %q to be rejected", value)
		}
	}
}

func TestAssistantSearchPublicIPPolicy(t *testing.T) {
	cases := []struct {
		address string
		public  bool
	}{
		{address: "8.8.8.8", public: true},
		{address: "2001:4860:4860::8888", public: true},
		{address: "10.0.0.1", public: false},
		{address: "100.64.0.1", public: false},
		{address: "192.0.2.1", public: false},
		{address: "fd00::1", public: false},
	}
	for _, testCase := range cases {
		ip := net.ParseIP(testCase.address)
		if got := IsAssistantSearchPublicIP(ip); got != testCase.public {
			t.Fatalf("IsAssistantSearchPublicIP(%q) = %t, want %t", testCase.address, got, testCase.public)
		}
	}
}
