package controller

import (
	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/gin-gonic/gin"
)

func GetOpenSourceBountyMCPToken(c *gin.Context) {
	status, err := model.GetOpenSourceBountyMCPTokenStatus(c.GetInt("id"))
	if err != nil {
		openSourceBountyApiError(c, err)
		return
	}
	common.ApiSuccess(c, gin.H{
		"status": status, "endpoint": "/mcp", "protocol_version": openSourceBountyMCPProtocolVersion,
	})
}

func RotateOpenSourceBountyMCPToken(c *gin.Context) {
	token, status, err := model.RotateOpenSourceBountyMCPToken(c.GetInt("id"))
	if err != nil {
		openSourceBountyApiError(c, err)
		return
	}
	common.ApiSuccess(c, gin.H{
		"token": token, "status": status, "endpoint": "/mcp", "protocol_version": openSourceBountyMCPProtocolVersion,
	})
}

func RevokeOpenSourceBountyMCPToken(c *gin.Context) {
	if err := model.RevokeOpenSourceBountyMCPToken(c.GetInt("id")); err != nil {
		openSourceBountyApiError(c, err)
		return
	}
	common.ApiSuccess(c, nil)
}
