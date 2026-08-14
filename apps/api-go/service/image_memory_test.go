package service

import (
	"io"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/stretchr/testify/require"
)

func TestImageConfigInspectionIsBounded(t *testing.T) {
	_, _, err := getImageConfig(io.LimitReader(zeroReader{}, imageConfigReadLimit+1))
	require.ErrorIs(t, err, common.ErrLimitExceeded)
}

type zeroReader struct{}

func (zeroReader) Read(data []byte) (int, error) {
	for i := range data {
		data[i] = 0
	}
	return len(data), nil
}
