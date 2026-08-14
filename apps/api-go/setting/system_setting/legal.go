package system_setting

import (
	"strings"

	"github.com/LIghtJUNction/api.lmm.best/setting/config"
)

type LegalSettings struct {
	UserAgreement string `json:"user_agreement"`
	PrivacyPolicy string `json:"privacy_policy"`
}

var defaultLegalSettings LegalSettings

func init() {
	config.GlobalConfig.Register("legal", &defaultLegalSettings)
}

func GetLegalSettings() *LegalSettings {
	return &defaultLegalSettings
}

func UserAgreementPublished() bool {
	return strings.TrimSpace(defaultLegalSettings.UserAgreement) != ""
}

func PrivacyPolicyPublished() bool {
	return strings.TrimSpace(defaultLegalSettings.PrivacyPolicy) != ""
}

func RegistrationPoliciesPublished() bool {
	return UserAgreementPublished() && PrivacyPolicyPublished()
}
