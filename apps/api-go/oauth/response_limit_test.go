package oauth

import (
	"io"
	"strings"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/stretchr/testify/require"
)

func TestDecodeOAuthJSONRejectsOversizedProviderResponse(t *testing.T) {
	body := `{"id":1,"name":"` + strings.Repeat("x", int(oauthResponseBodyMaxBytes)) + `"}`
	var profile struct {
		ID int `json:"id"`
	}

	err := decodeOAuthJSON(strings.NewReader(body), &profile)
	require.ErrorIs(t, err, common.ErrLimitExceeded)
}

func TestDecodeOAuthJSONAcceptsSmallProviderResponse(t *testing.T) {
	var profile struct {
		ID int `json:"id"`
	}

	err := decodeOAuthJSON(io.Reader(strings.NewReader(`{"id":42}`)), &profile)
	require.NoError(t, err)
	require.Equal(t, 42, profile.ID)
}
