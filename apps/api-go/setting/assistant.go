package setting

import (
	"errors"
	"net"
	"net/url"
	"strconv"
	"strings"
	"sync"
)

const (
	AssistantEnabledOptionKey = "AssistantEnabled"
	AssistantModelOptionKey   = "AssistantModel"
	// AssistantWeeklyCreditUSDOptionKey is retained only so older consoles can
	// read and submit their retired field without affecting runtime funding.
	AssistantWeeklyCreditUSDOptionKey        = "AssistantWeeklyCreditUSD"
	AssistantAgentLoopEnabledOptionKey       = "AssistantAgentLoopEnabled"
	AssistantMaxStepsOptionKey               = "AssistantMaxSteps"
	AssistantTimeoutSecondsOptionKey         = "AssistantTimeoutSeconds"
	AssistantCacheEnabledOptionKey           = "AssistantCacheEnabled"
	AssistantCacheTTLMinutesOptionKey        = "AssistantCacheTTLMinutes"
	AssistantPersonaOptionKey                = "AssistantPersona"
	AssistantSystemPromptOptionKey           = "AssistantSystemPrompt"
	AssistantSearchProviderOptionKey         = "AssistantSearchProvider"
	AssistantSearchURLOptionKey              = "AssistantSearchURL"
	AssistantSearchAPIKeyOptionKey           = "AssistantSearchAPIKey"
	AssistantSearchMCPToolOptionKey          = "AssistantSearchMCPTool"
	AssistantSkillsOptionKey                 = "AssistantSkills"
	AssistantReviewEnabledOptionKey          = "AssistantReviewEnabled"
	AssistantReviewWindowDaysOptionKey       = "AssistantReviewWindowDays"
	AssistantReviewIntervalHoursOptionKey    = "AssistantReviewIntervalHours"
	AssistantRetentionEnabledOptionKey       = "AssistantRetentionEnabled"
	AssistantActiveRetentionDaysOptionKey    = "AssistantActiveRetentionDays"
	AssistantArchivedRetentionDaysOptionKey  = "AssistantArchivedRetentionDays"
	AssistantSecurityRetentionDaysOptionKey  = "AssistantSecurityRetentionDays"
	AssistantRetentionIntervalHoursOptionKey = "AssistantRetentionIntervalHours"
	DefaultAssistantModel                    = "deepseek-v4-flash"
)

type AssistantSearchProvider string

const (
	AssistantSearchProviderNone              AssistantSearchProvider = "none"
	AssistantSearchProviderExa               AssistantSearchProvider = "exa"
	AssistantSearchProviderTavily            AssistantSearchProvider = "tavily"
	AssistantSearchProviderBrave             AssistantSearchProvider = "brave"
	AssistantSearchProviderGenericHTTP       AssistantSearchProvider = "generic_http"
	AssistantSearchProviderMCPStreamableHTTP AssistantSearchProvider = "mcp_streamable_http"
	// DefaultAssistantSearchProvider keeps installations that already have a
	// SearchURL working after the provider selector is introduced.
	DefaultAssistantSearchProvider = AssistantSearchProviderGenericHTTP
)

type AssistantSettings struct {
	Enabled                bool
	Model                  string
	AgentLoopEnabled       bool
	MaxSteps               int
	TimeoutSeconds         int
	CacheEnabled           bool
	CacheTTLMinutes        int
	Persona                string
	SystemPrompt           string
	SearchProvider         AssistantSearchProvider
	SearchURL              string
	SearchAPIKey           string
	SearchMCPTool          string
	Skills                 string
	ReviewEnabled          bool
	ReviewWindowDays       int
	ReviewIntervalHours    int
	RetentionEnabled       bool
	ActiveRetentionDays    int
	ArchivedRetentionDays  int
	SecurityRetentionDays  int
	RetentionIntervalHours int
}

var (
	assistantSettingsMutex sync.RWMutex
	assistantSettings      = AssistantSettings{
		Enabled:                true,
		Model:                  DefaultAssistantModel,
		AgentLoopEnabled:       true,
		MaxSteps:               6,
		TimeoutSeconds:         45,
		CacheEnabled:           true,
		CacheTTLMinutes:        1440,
		Persona:                "",
		SystemPrompt:           "",
		SearchProvider:         DefaultAssistantSearchProvider,
		SearchURL:              "",
		SearchAPIKey:           "",
		SearchMCPTool:          "",
		Skills:                 "",
		ReviewEnabled:          true,
		ReviewWindowDays:       30,
		ReviewIntervalHours:    24,
		RetentionEnabled:       true,
		ActiveRetentionDays:    90,
		ArchivedRetentionDays:  30,
		SecurityRetentionDays:  180,
		RetentionIntervalHours: 24,
	}
)

func GetAssistantSettings() AssistantSettings {
	assistantSettingsMutex.RLock()
	defer assistantSettingsMutex.RUnlock()
	return assistantSettings
}

func SetAssistantEnabled(enabled bool) {
	assistantSettingsMutex.Lock()
	defer assistantSettingsMutex.Unlock()
	assistantSettings.Enabled = enabled
}

func UpdateAssistantModel(value string) error {
	model := strings.TrimSpace(value)
	if model == "" {
		return errors.New("assistant model is required")
	}
	if len(model) > 128 {
		return errors.New("assistant model must be at most 128 characters")
	}

	assistantSettingsMutex.Lock()
	defer assistantSettingsMutex.Unlock()
	assistantSettings.Model = model
	return nil
}

func SetAssistantAgentLoopEnabled(enabled bool) {
	assistantSettingsMutex.Lock()
	defer assistantSettingsMutex.Unlock()
	assistantSettings.AgentLoopEnabled = enabled
}

func UpdateAssistantMaxSteps(value string) error {
	steps, err := strconv.Atoi(strings.TrimSpace(value))
	if err != nil || steps < 1 || steps > 12 {
		return errors.New("assistant max steps must be between 1 and 12")
	}

	assistantSettingsMutex.Lock()
	defer assistantSettingsMutex.Unlock()
	assistantSettings.MaxSteps = steps
	return nil
}

func UpdateAssistantTimeoutSeconds(value string) error {
	seconds, err := strconv.Atoi(strings.TrimSpace(value))
	if err != nil || seconds < 5 || seconds > 120 {
		return errors.New("assistant timeout must be between 5 and 120 seconds")
	}

	assistantSettingsMutex.Lock()
	defer assistantSettingsMutex.Unlock()
	assistantSettings.TimeoutSeconds = seconds
	return nil
}

func SetAssistantCacheEnabled(enabled bool) {
	assistantSettingsMutex.Lock()
	defer assistantSettingsMutex.Unlock()
	assistantSettings.CacheEnabled = enabled
}

func UpdateAssistantCacheTTLMinutes(value string) error {
	minutes, err := strconv.Atoi(strings.TrimSpace(value))
	if err != nil || minutes < 0 || minutes > 10080 {
		return errors.New("assistant cache TTL must be between 0 and 10080 minutes")
	}

	assistantSettingsMutex.Lock()
	defer assistantSettingsMutex.Unlock()
	assistantSettings.CacheTTLMinutes = minutes
	return nil
}

func updateAssistantText(target *string, value string, maxLength int, message string) error {
	value = strings.TrimSpace(value)
	if len([]rune(value)) > maxLength {
		return errors.New(message)
	}
	assistantSettingsMutex.Lock()
	defer assistantSettingsMutex.Unlock()
	*target = value
	return nil
}

func UpdateAssistantPersona(value string) error {
	return updateAssistantText(&assistantSettings.Persona, value, 2000, "assistant persona must be at most 2000 characters")
}

func UpdateAssistantSystemPrompt(value string) error {
	return updateAssistantText(&assistantSettings.SystemPrompt, value, 8000, "assistant system prompt must be at most 8000 characters")
}

func UpdateAssistantSearchProvider(value string) error {
	provider := AssistantSearchProvider(strings.TrimSpace(value))
	if !IsAssistantSearchProvider(provider) {
		return errors.New("assistant search provider is invalid")
	}
	assistantSettingsMutex.Lock()
	defer assistantSettingsMutex.Unlock()
	assistantSettings.SearchProvider = provider
	return nil
}

func UpdateAssistantSearchURL(value string) error {
	if err := ValidateAssistantSearchURL(value); err != nil {
		return err
	}
	return updateAssistantText(&assistantSettings.SearchURL, value, 512, "assistant search URL must be at most 512 characters")
}

func UpdateAssistantSearchAPIKey(value string) error {
	return updateAssistantText(&assistantSettings.SearchAPIKey, value, 512, "assistant search API key must be at most 512 characters")
}

func UpdateAssistantSearchMCPTool(value string) error {
	return updateAssistantText(&assistantSettings.SearchMCPTool, value, 128, "assistant search MCP tool must be at most 128 characters")
}

func UpdateAssistantSkills(value string) error {
	return updateAssistantText(&assistantSettings.Skills, value, 12000, "assistant skills must be at most 12000 characters")
}

func SetAssistantReviewEnabled(enabled bool) {
	assistantSettingsMutex.Lock()
	defer assistantSettingsMutex.Unlock()
	assistantSettings.ReviewEnabled = enabled
}

func UpdateAssistantReviewWindowDays(value string) error {
	return updateAssistantNumber(&assistantSettings.ReviewWindowDays, value, 1, 90, "assistant review window must be between 1 and 90 days")
}

func UpdateAssistantReviewIntervalHours(value string) error {
	return updateAssistantNumber(&assistantSettings.ReviewIntervalHours, value, 1, 168, "assistant review interval must be between 1 and 168 hours")
}

func SetAssistantRetentionEnabled(enabled bool) {
	assistantSettingsMutex.Lock()
	defer assistantSettingsMutex.Unlock()
	assistantSettings.RetentionEnabled = enabled
}

func updateAssistantNumber(target *int, value string, minimum, maximum int, message string) error {
	number, err := strconv.Atoi(strings.TrimSpace(value))
	if err != nil || number < minimum || number > maximum {
		return errors.New(message)
	}
	assistantSettingsMutex.Lock()
	defer assistantSettingsMutex.Unlock()
	*target = number
	return nil
}

func UpdateAssistantActiveRetentionDays(value string) error {
	return updateAssistantNumber(&assistantSettings.ActiveRetentionDays, value, 7, 3650, "assistant active retention must be between 7 and 3650 days")
}

func UpdateAssistantArchivedRetentionDays(value string) error {
	return updateAssistantNumber(&assistantSettings.ArchivedRetentionDays, value, 1, 3650, "assistant archived retention must be between 1 and 3650 days")
}

func UpdateAssistantSecurityRetentionDays(value string) error {
	return updateAssistantNumber(&assistantSettings.SecurityRetentionDays, value, 30, 3650, "assistant security retention must be between 30 and 3650 days")
}

func UpdateAssistantRetentionIntervalHours(value string) error {
	return updateAssistantNumber(&assistantSettings.RetentionIntervalHours, value, 1, 168, "assistant retention interval must be between 1 and 168 hours")
}

func ValidateAssistantOption(key string, value string) error {
	switch key {
	case AssistantReviewEnabledOptionKey:
		if _, err := strconv.ParseBool(strings.TrimSpace(value)); err != nil {
			return errors.New("assistant review enabled must be a boolean")
		}
	case AssistantModelOptionKey:
		model := strings.TrimSpace(value)
		if model == "" {
			return errors.New("assistant model is required")
		}
		if len(model) > 128 {
			return errors.New("assistant model must be at most 128 characters")
		}
	case AssistantMaxStepsOptionKey:
		steps, err := strconv.Atoi(strings.TrimSpace(value))
		if err != nil || steps < 1 || steps > 12 {
			return errors.New("assistant max steps must be between 1 and 12")
		}
	case AssistantTimeoutSecondsOptionKey:
		seconds, err := strconv.Atoi(strings.TrimSpace(value))
		if err != nil || seconds < 5 || seconds > 120 {
			return errors.New("assistant timeout must be between 5 and 120 seconds")
		}
	case AssistantCacheTTLMinutesOptionKey:
		minutes, err := strconv.Atoi(strings.TrimSpace(value))
		if err != nil || minutes < 0 || minutes > 10080 {
			return errors.New("assistant cache TTL must be between 0 and 10080 minutes")
		}
	case AssistantPersonaOptionKey:
		if len([]rune(strings.TrimSpace(value))) > 2000 {
			return errors.New("assistant persona must be at most 2000 characters")
		}
	case AssistantSystemPromptOptionKey:
		if len([]rune(strings.TrimSpace(value))) > 8000 {
			return errors.New("assistant system prompt must be at most 8000 characters")
		}
	case AssistantSearchProviderOptionKey:
		if !IsAssistantSearchProvider(AssistantSearchProvider(strings.TrimSpace(value))) {
			return errors.New("assistant search provider is invalid")
		}
	case AssistantSearchURLOptionKey:
		return ValidateAssistantSearchURL(value)
	case AssistantSearchAPIKeyOptionKey:
		if len([]rune(strings.TrimSpace(value))) > 512 {
			return errors.New("assistant search API key must be at most 512 characters")
		}
	case AssistantSearchMCPToolOptionKey:
		if len([]rune(strings.TrimSpace(value))) > 128 {
			return errors.New("assistant search MCP tool must be at most 128 characters")
		}
	case AssistantSkillsOptionKey:
		if len([]rune(strings.TrimSpace(value))) > 12000 {
			return errors.New("assistant skills must be at most 12000 characters")
		}
	case AssistantReviewWindowDaysOptionKey:
		return validateAssistantNumber(value, 1, 90, "assistant review window must be between 1 and 90 days")
	case AssistantReviewIntervalHoursOptionKey:
		return validateAssistantNumber(value, 1, 168, "assistant review interval must be between 1 and 168 hours")
	case AssistantActiveRetentionDaysOptionKey:
		return validateAssistantNumber(value, 7, 3650, "assistant active retention must be between 7 and 3650 days")
	case AssistantArchivedRetentionDaysOptionKey:
		return validateAssistantNumber(value, 1, 3650, "assistant archived retention must be between 1 and 3650 days")
	case AssistantSecurityRetentionDaysOptionKey:
		return validateAssistantNumber(value, 30, 3650, "assistant security retention must be between 30 and 3650 days")
	case AssistantRetentionIntervalHoursOptionKey:
		return validateAssistantNumber(value, 1, 168, "assistant retention interval must be between 1 and 168 hours")
	}
	return nil
}

func validateAssistantNumber(value string, minimum, maximum int, message string) error {
	number, err := strconv.Atoi(strings.TrimSpace(value))
	if err != nil || number < minimum || number > maximum {
		return errors.New(message)
	}
	return nil
}

func IsAssistantSearchProvider(provider AssistantSearchProvider) bool {
	switch provider {
	case AssistantSearchProviderNone,
		AssistantSearchProviderExa,
		AssistantSearchProviderTavily,
		AssistantSearchProviderBrave,
		AssistantSearchProviderGenericHTTP,
		AssistantSearchProviderMCPStreamableHTTP:
		return true
	default:
		return false
	}
}

// ValidateAssistantSearchURL checks the administrator-supplied search
// endpoint's syntax and rejects address literals that cannot be a public
// search provider. Hostnames are checked again at connection time because DNS
// answers can change after an option is saved.
func ValidateAssistantSearchURL(value string) error {
	value = strings.TrimSpace(value)
	if value == "" {
		return nil
	}
	parsed, err := url.ParseRequestURI(value)
	if err != nil || (parsed.Scheme != "http" && parsed.Scheme != "https") || parsed.Host == "" {
		return errors.New("assistant search URL must be an absolute HTTP or HTTPS URL")
	}
	if parsed.User != nil {
		return errors.New("assistant search URL must not contain embedded credentials")
	}
	hostname := strings.TrimSuffix(strings.ToLower(parsed.Hostname()), ".")
	if hostname == "" {
		return errors.New("assistant search URL must include a host")
	}
	if ip := net.ParseIP(hostname); ip != nil && !IsAssistantSearchPublicIP(ip) {
		return errors.New("assistant search URL must resolve to a public address")
	}
	return nil
}

func IsAssistantSearchPublicIP(ip net.IP) bool {
	if ip == nil || !ip.IsGlobalUnicast() || ip.IsPrivate() || ip.IsLoopback() || ip.IsUnspecified() || ip.IsLinkLocalUnicast() || ip.IsLinkLocalMulticast() || ip.IsMulticast() {
		return false
	}
	if ip4 := ip.To4(); ip4 != nil {
		// Carrier-grade NAT, benchmarking, documentation, and reserved ranges
		// are not public service addresses even though some are global unicast.
		if ip4[0] == 100 && ip4[1] >= 64 && ip4[1] <= 127 {
			return false
		}
		if ip4[0] == 192 && ip4[1] == 0 && ip4[2] == 0 {
			return false
		}
		if ip4[0] == 192 && ip4[1] == 0 && ip4[2] == 2 {
			return false
		}
		if ip4[0] == 198 && ip4[1] == 18 {
			return false
		}
		if ip4[0] == 198 && ip4[1] == 19 {
			return false
		}
		if ip4[0] == 198 && ip4[1] == 51 && ip4[2] == 100 {
			return false
		}
		if ip4[0] == 203 && ip4[1] == 0 && ip4[2] == 113 {
			return false
		}
	}
	return true
}
