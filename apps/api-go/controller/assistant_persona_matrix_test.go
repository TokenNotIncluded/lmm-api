package controller

import (
	_ "embed"
	"encoding/json"
	"strings"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

// Keep the personas in data so the same matrix can be reviewed, extended, or
// replayed by a small simulator without changing the assertions themselves.
//
//go:embed assistant_persona_matrix_testdata.json
var assistantPersonaMatrixTestdata []byte

type assistantPersonaMatrixFixture struct {
	ID       string                      `json:"id"`
	Label    string                      `json:"label"`
	User     assistantPersonaUserFixture `json:"user"`
	Message  string                      `json:"message"`
	Expected assistantPersonaExpectation `json:"expected"`
}

type assistantPersonaUserFixture struct {
	Username               string `json:"username"`
	Email                  string `json:"email"`
	AccessLevel            string `json:"access_level"`
	AdministratorMode      bool   `json:"administrator_mode"`
	DeveloperAccessGranted bool   `json:"developer_access_granted"`
	PaymentMethodsHidden   bool   `json:"payment_methods_hidden"`
}

type assistantPersonaExpectation struct {
	Profile         string                              `json:"profile"`
	Signals         []string                            `json:"signals"`
	WelcomeContains []string                            `json:"welcome_contains"`
	Context         assistantPersonaContextExpectation  `json:"context"`
	Security        assistantPersonaSecurityExpectation `json:"security"`
}

type assistantPersonaContextExpectation struct {
	MaskedEmail          string `json:"masked_email"`
	EmailDomain          string `json:"email_domain"`
	EmailCategory        string `json:"email_category"`
	AccessLevel          string `json:"access_level"`
	PaymentMethodsHidden bool   `json:"payment_methods_hidden"`
}

type assistantPersonaSecurityExpectation struct {
	HighConfidenceAbuse    bool `json:"high_confidence_abuse"`
	AuthorizedSecurityTest bool `json:"authorized_security_test"`
}

func loadAssistantPersonaMatrix(t *testing.T) []assistantPersonaMatrixFixture {
	t.Helper()
	var fixtures []assistantPersonaMatrixFixture
	require.NoError(t, json.Unmarshal(assistantPersonaMatrixTestdata, &fixtures))
	require.GreaterOrEqual(t, len(fixtures), 9)
	return fixtures
}

func assistantPersonaContextFromFixture(fixture assistantPersonaUserFixture) assistantUserContext {
	context := assistantUserContext{
		Username:               strings.TrimSpace(fixture.Username),
		AccessLevel:            fixture.AccessLevel,
		AdministratorMode:      fixture.AdministratorMode,
		DeveloperAccessGranted: fixture.DeveloperAccessGranted,
		PaymentMethodsHidden:   fixture.PaymentMethodsHidden,
	}
	if context.AccessLevel == "" {
		context.AccessLevel = "L0"
	}

	rawEmail := strings.TrimSpace(fixture.Email)
	if rawEmail != "" {
		context.Email, context.EmailDomain = maskAssistantEmail(rawEmail)
		context.EmailCategory = classifyAssistantEmail(rawEmail)
	}
	return context
}

func TestAssistantPersonaMatrix(t *testing.T) {
	fixtures := loadAssistantPersonaMatrix(t)
	requiredIDs := map[string]bool{
		"A": false,
		"B": false,
		"C": false,
		"D": false,
		"E": false,
		"F": false,
		"G": false,
		"H": false,
	}
	seenIDs := make(map[string]struct{}, len(fixtures))

	for _, fixture := range fixtures {
		fixture := fixture
		t.Run(fixture.ID+"/"+fixture.Label, func(t *testing.T) {
			require.NotEmpty(t, fixture.ID)
			require.NotEmpty(t, fixture.Message)
			if _, exists := seenIDs[fixture.ID]; exists {
				t.Fatalf("duplicate persona id %q", fixture.ID)
			}
			seenIDs[fixture.ID] = struct{}{}
			if _, required := requiredIDs[fixture.ID]; required {
				requiredIDs[fixture.ID] = true
			}

			context := assistantPersonaContextFromFixture(fixture.User)
			profile, signals := classifyAssistantCustomerProfile(context, fixture.Message)
			assert.Equal(t, fixture.Expected.Profile, string(profile))
			for _, expectedSignal := range fixture.Expected.Signals {
				assert.Contains(t, signals, expectedSignal)
			}

			strategy := assistantWelcomeStrategy(profile)
			for _, expectedText := range fixture.Expected.WelcomeContains {
				assert.Contains(t, strategy, expectedText)
			}

			assert.Equal(t, fixture.Expected.Context.MaskedEmail, context.Email)
			assert.Equal(t, fixture.Expected.Context.EmailDomain, context.EmailDomain)
			assert.Equal(t, fixture.Expected.Context.EmailCategory, context.EmailCategory)
			assert.Equal(t, fixture.Expected.Context.AccessLevel, context.AccessLevel)
			assert.Equal(t, fixture.Expected.Context.PaymentMethodsHidden, context.PaymentMethodsHidden)

			isHighConfidenceAbuse := assistantHasHighConfidenceSecurityAbuse(fixture.Message)
			assert.Equal(t, fixture.Expected.Security.HighConfidenceAbuse, isHighConfidenceAbuse)
			if fixture.Expected.Security.AuthorizedSecurityTest {
				assert.False(t, isHighConfidenceAbuse, "authorized non-destructive security guidance must not be hard-refused")
			}
		})
	}

	for id, covered := range requiredIDs {
		assert.True(t, covered, "required persona %s is missing from the fixture", id)
	}
}

func TestAssistantPersonaContextRedactsIdentitySecrets(t *testing.T) {
	context := assistantPersonaContextFromFixture(assistantPersonaUserFixture{
		Username:             "privacy-user",
		Email:                "person@example.com",
		AccessLevel:          "L1",
		PaymentMethodsHidden: true,
	})

	serialized, err := json.Marshal(context)
	require.NoError(t, err)
	assert.Equal(t, "pe***n@example.com", context.Email)
	assert.NotContains(t, string(serialized), "person@example.com")
	assert.NotContains(t, string(serialized), "password")
	assert.NotContains(t, string(serialized), "sk-")
	assert.Contains(t, string(serialized), "privacy-user")
}
