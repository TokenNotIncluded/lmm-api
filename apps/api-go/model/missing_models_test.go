package model

import (
	"testing"

	"github.com/stretchr/testify/require"
)

func TestGetMissingModelsPropagatesEnabledAbilityQueryError(t *testing.T) {
	// The console setup database deliberately has no abilities table. A
	// missing table must be reported as unavailable rather than being turned
	// into an empty model inventory that an administrator could act on.
	setupConsoleActivationTestDB(t)

	missing, err := GetMissingModels()
	require.Error(t, err)
	require.Nil(t, missing)
}
