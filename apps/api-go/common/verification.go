package common

import (
	"strings"
	"time"

	"github.com/LIghtJUNction/api.lmm.best/pkg/cachex"
	"github.com/google/uuid"
)

type verificationValue struct {
	code string
}

const (
	EmailVerificationPurpose = "v"
	// SecurityEmailVerificationPurpose is kept separate from registration and
	// email-binding codes so a code requested for a sensitive action cannot be
	// replayed in an account-creation flow (or vice versa).
	SecurityEmailVerificationPurpose = "s"
	PasswordResetPurpose             = "r"

	verificationMaxEntries = 16_384
	verificationMaxBytes   = 2 << 20
)

var (
	VerificationValidMinutes = 10
	verificationCodes        = cachex.NewByteCache[verificationValue](verificationMaxEntries, verificationMaxBytes, func(key string, value verificationValue) int64 {
		return int64(len(key) + len(value.code) + 16)
	})
)

func GenerateVerificationCode(length int) string {
	code := strings.ReplaceAll(uuid.New().String(), "-", "")
	if length == 0 {
		return code
	}
	return code[:length]
}

func RegisterVerificationCodeWithKey(key string, code string, purpose string) {
	verificationCodes.SetWithTTL(purpose+key, verificationValue{code: code}, time.Duration(VerificationValidMinutes)*time.Minute)
}

func VerifyCodeWithKey(key string, code string, purpose string) bool {
	value, found := verificationCodes.Load(purpose + key)
	return found && code == value.code
}

func DeleteKey(key string, purpose string) {
	verificationCodes.Delete(purpose + key)
}
