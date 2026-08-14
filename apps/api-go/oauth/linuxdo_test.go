package oauth

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestLinuxDOUserInfoIncludesPublicGamificationScore(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/api/user":
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"id":42,"username":"score-user","name":"Score User","active":true,"trust_level":3,"silenced":false}`))
		case "/u/score-user.json":
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"user":{"gamification_score":10000.5}}`))
		default:
			http.NotFound(w, r)
		}
	}))
	t.Cleanup(server.Close)
	t.Setenv("LINUX_DO_USER_ENDPOINT", server.URL+"/api/user")
	t.Setenv("LINUX_DO_PROFILE_ENDPOINT", server.URL+"/u")
	previousMinimumTrustLevel := common.LinuxDOMinimumTrustLevel
	common.LinuxDOMinimumTrustLevel = 0
	t.Cleanup(func() { common.LinuxDOMinimumTrustLevel = previousMinimumTrustLevel })

	user, err := (&LinuxDOProvider{}).GetUserInfo(context.Background(), &OAuthToken{AccessToken: "test-token"})
	require.NoError(t, err)
	assert.Equal(t, "42", user.ProviderUserID)
	assert.Equal(t, 10000.5, user.Extra["gamification_score"])
}

func TestLinuxDOUserInfoPrefersConnectScore(t *testing.T) {
	profileRequested := false
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/user" {
			profileRequested = true
			http.NotFound(w, r)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":43,"username":"direct-score","active":true,"trust_level":3,"gamification_score":12001}`))
	}))
	t.Cleanup(server.Close)
	t.Setenv("LINUX_DO_USER_ENDPOINT", server.URL+"/api/user")
	t.Setenv("LINUX_DO_PROFILE_ENDPOINT", server.URL+"/u")
	previousMinimumTrustLevel := common.LinuxDOMinimumTrustLevel
	common.LinuxDOMinimumTrustLevel = 0
	t.Cleanup(func() { common.LinuxDOMinimumTrustLevel = previousMinimumTrustLevel })

	user, err := (&LinuxDOProvider{}).GetUserInfo(context.Background(), &OAuthToken{AccessToken: "test-token"})
	require.NoError(t, err)
	assert.Equal(t, float64(12001), user.Extra["gamification_score"])
	assert.False(t, profileRequested)
}
