package controller

import (
	"net/http"

	"github.com/LIghtJUNction/api.lmm.best/setting/system_setting"
	"github.com/gin-gonic/gin"
)

const (
	registrationLegalUnavailableCode    = "REGISTRATION_LEGAL_UNAVAILABLE"
	registrationLegalUnavailableMessage = "Registration is unavailable until the user agreement and privacy policy are published."
	legalConsentRequiredCode            = "LEGAL_CONSENT_REQUIRED"
	legalConsentRequiredMessage         = "You must accept the user agreement and privacy policy to register."
)

type registrationGateError struct {
	Status  int
	Code    string
	Message string
}

func (err *registrationGateError) Error() string {
	return err.Message
}

func registrationGateFailure(policiesPublished, acceptedLegal bool) *registrationGateError {
	if !policiesPublished {
		return &registrationGateError{
			Status:  http.StatusServiceUnavailable,
			Code:    registrationLegalUnavailableCode,
			Message: registrationLegalUnavailableMessage,
		}
	}
	if !acceptedLegal {
		return &registrationGateError{
			Status:  http.StatusUnprocessableEntity,
			Code:    legalConsentRequiredCode,
			Message: legalConsentRequiredMessage,
		}
	}
	return nil
}

func publicRegistrationGateFailure(acceptedLegal bool) *registrationGateError {
	return registrationGateFailure(system_setting.RegistrationPoliciesPublished(), acceptedLegal)
}

func writeRegistrationGateError(c *gin.Context, err *registrationGateError) {
	c.JSON(err.Status, gin.H{
		"success": false,
		"code":    err.Code,
		"message": err.Message,
	})
}

func requirePublicRegistrationLegal(c *gin.Context, acceptedLegal bool) bool {
	err := publicRegistrationGateFailure(acceptedLegal)
	if err == nil {
		return true
	}
	writeRegistrationGateError(c, err)
	return false
}
