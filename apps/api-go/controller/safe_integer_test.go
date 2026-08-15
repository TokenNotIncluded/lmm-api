package controller

import (
	"strconv"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/stretchr/testify/require"
)

func TestParsePlatformUintRejectsValuesThatWouldWrap(t *testing.T) {
	_, err := parsePlatformUint("18446744073709551616")
	require.Error(t, err)

	channel := &model.Channel{}
	_, err = applyAssistantAdminChannelField(channel, "weight", "18446744073709551616")
	require.Error(t, err)
	_, err = parsePlatformUint(strconv.FormatUint(uint64(^uint(0)), 10))
	require.NoError(t, err)
}
