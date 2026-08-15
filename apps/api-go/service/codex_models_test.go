package service

import (
	"testing"

	"github.com/stretchr/testify/require"
)

func TestCodexModelsURLUsesOnlyOfficialOrigins(t *testing.T) {
	modelsURL, err := codexModelsURL("https://chatgpt.com/", "1.2.3")
	require.NoError(t, err)
	require.Equal(t, "https://chatgpt.com/backend-api/codex/models?client_version=1.2.3", modelsURL.String())

	for _, baseURL := range []string{
		"http://chatgpt.com",
		"https://127.0.0.1",
		"https://attacker.example",
		"https://chatgpt.com:8443",
		"https://chatgpt.com/internal",
		"https://chatgpt.com/?target=internal",
	} {
		_, err := codexModelsURL(baseURL, "1.2.3")
		require.Error(t, err, baseURL)
	}
}
