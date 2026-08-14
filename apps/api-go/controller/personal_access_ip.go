package controller

import (
	"errors"
	"net"
	"net/http"
	"net/netip"
	"strings"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/middleware"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type personalAccessIPRequest struct {
	IP string `json:"ip"`
}

type personalAccessIPResponse struct {
	IP                  string `json:"ip"`
	CurrentIP           string `json:"current_ip"`
	CurrentIPAllowed    bool   `json:"current_ip_allowed"`
	Eligible            bool   `json:"eligible"`
	MinimumTrustLevel   int    `json:"minimum_trust_level"`
	ProductionCNLinkage bool   `json:"production_cn_linkage"`
}

func personalAccessIPError(c *gin.Context, status int, code string, message string) {
	c.AbortWithStatusJSON(status, gin.H{
		"success": false,
		"code":    code,
		"message": message,
	})
}

func personalAccessIPPolicyError(c *gin.Context, status int, code string, message string) {
	c.Header(accessPolicyResultHeader, accessPolicyDenied)
	personalAccessIPError(c, status, code, message)
}

func currentUserForPersonalAccessIP(c *gin.Context) (*model.User, error) {
	userID := c.GetInt("id")
	if userID <= 0 {
		return nil, gorm.ErrInvalidData
	}
	return model.GetUserById(userID, false)
}

func personalAccessIPResponseForUser(user *model.User, currentIP string) (personalAccessIPResponse, error) {
	trust, err := model.GetFreshTrustLevelInfoForUser(user)
	if err != nil {
		return personalAccessIPResponse{}, err
	}
	record, err := model.GetPersonalAccessIP(user.Id)
	if err != nil {
		return personalAccessIPResponse{}, err
	}
	response := personalAccessIPResponse{
		CurrentIP:           currentIP,
		Eligible:            trust.Level >= model.PersonalAccessIPMinTrustLevel,
		MinimumTrustLevel:   model.PersonalAccessIPMinTrustLevel,
		ProductionCNLinkage: true,
	}
	if record != nil {
		response.IP = record.IP
	}
	if currentIP != "" && response.IP != "" {
		response.CurrentIPAllowed = response.IP == currentIP && response.Eligible && user.Status == common.UserStatusEnabled
	}
	return response, nil
}

func GetPersonalAccessIP(c *gin.Context) {
	user, err := currentUserForPersonalAccessIP(c)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	currentIP, _ := model.NormalizePersonalAccessIP(c.ClientIP())
	response, err := personalAccessIPResponseForUser(user, currentIP)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, response)
}

func SetPersonalAccessIP(c *gin.Context) {
	user, err := currentUserForPersonalAccessIP(c)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	var request personalAccessIPRequest
	if err := c.ShouldBindJSON(&request); err != nil {
		personalAccessIPError(c, http.StatusUnprocessableEntity, "INVALID_IP", "a public IP address is required")
		return
	}
	_, err = model.SetPersonalAccessIP(user, request.IP)
	if err != nil {
		switch {
		case errors.Is(err, model.ErrPersonalAccessIPNotEligible):
			personalAccessIPError(c, http.StatusForbidden, "TRUST_LEVEL_REQUIRED", "personal IP allowlist requires trust level L1 or higher")
		case errors.Is(err, model.ErrInvalidPersonalAccessIP):
			personalAccessIPError(c, http.StatusUnprocessableEntity, "INVALID_IP", "IP address must be public and globally routable")
		default:
			common.ApiError(c, err)
		}
		return
	}
	currentIP, _ := model.NormalizePersonalAccessIP(c.ClientIP())
	response, err := personalAccessIPResponseForUser(user, currentIP)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, response)
}

func DeletePersonalAccessIP(c *gin.Context) {
	if err := model.DeletePersonalAccessIP(c.GetInt("id")); err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, nil)
}

func loopbackPeer(remoteAddr string) bool {
	if host, _, err := net.SplitHostPort(strings.TrimSpace(remoteAddr)); err == nil {
		remoteAddr = host
	}
	addr, err := netip.ParseAddr(strings.TrimSpace(remoteAddr))
	return err == nil && addr.IsLoopback()
}

// CheckPersonalAccessIPPolicy is consumed only by Nginx auth_request. The
// handler requires a loopback peer and takes the original client address from
// a header set by the local Nginx policy, never from an arbitrary public call.
func CheckPersonalAccessIPPolicy(c *gin.Context) {
	if !loopbackPeer(c.Request.RemoteAddr) {
		personalAccessIPPolicyError(c, http.StatusForbidden, "INTERNAL_ONLY", "internal policy endpoint")
		return
	}
	// The edge policy is always wired, but the administrator can disable the
	// enforcement (or choose another country set) from System Settings. This
	// keeps the decision in Go and avoids an Nginx/app configuration split.
	edgeCountry := strings.ToUpper(strings.TrimSpace(c.GetHeader("X-LMM-Edge-Country")))
	if edgeCountry == "" && strings.TrimSpace(c.GetHeader("X-LMM-CN-Source")) == "1" {
		edgeCountry = "CN"
	}
	if !common.IsRegionBlocked(edgeCountry) {
		c.Status(http.StatusNoContent)
		return
	}
	user, authenticated := middleware.AuthenticatedDashboardUser(c)
	if !authenticated || user == nil {
		personalAccessIPPolicyError(c, http.StatusUnauthorized, "AUTH_REQUIRED", "a valid account session is required")
		return
	}
	originalIP := strings.TrimSpace(c.GetHeader("X-Original-Client-IP"))
	if originalIP == "" {
		personalAccessIPPolicyError(c, http.StatusForbidden, "CLIENT_IP_REQUIRED", "original client IP is required")
		return
	}
	allowed, err := model.IsPersonalAccessIPAllowedForUser(user.Id, originalIP)
	if err != nil {
		common.SysError("personal access IP policy lookup failed: " + err.Error())
		personalAccessIPPolicyError(c, http.StatusForbidden, "POLICY_UNAVAILABLE", "access policy unavailable")
		return
	}
	if !allowed {
		personalAccessIPPolicyError(c, http.StatusForbidden, "CN_DIRECT_ACCESS_BLOCKED", "direct access is not allowed for this account")
		return
	}
	c.Status(http.StatusNoContent)
}
