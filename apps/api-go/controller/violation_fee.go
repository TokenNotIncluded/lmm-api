package controller

import (
	"errors"
	"net/http"
	"strconv"
	"strings"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/service"
	"github.com/gin-gonic/gin"
)

type violationFeeAppealInput struct {
	RecordID uint   `json:"record_id"`
	Reason   string `json:"reason"`
}

type violationFeeAppealReviewInput struct {
	Note string `json:"note"`
}

func SubmitViolationFeeAppeal(c *gin.Context) {
	var input violationFeeAppealInput
	if err := c.ShouldBindJSON(&input); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"success": false, "code": "VIOLATION_FEE_APPEAL_INVALID", "message": "申诉格式无效"})
		return
	}
	appeal, err := model.SubmitViolationFeeAppeal(c.GetInt("id"), input.RecordID, input.Reason)
	if err != nil {
		status := http.StatusBadRequest
		if errors.Is(err, model.ErrViolationFeeRecordNotFound) || errors.Is(err, model.ErrViolationFeeAppealState) {
			status = http.StatusConflict
		} else if errors.Is(err, model.ErrViolationFeeAppealPending) {
			status = http.StatusConflict
		}
		c.JSON(status, gin.H{"success": false, "code": "VIOLATION_FEE_APPEAL_REJECTED", "message": err.Error()})
		return
	}
	service.NotifyRootUser("violation_fee_appeal", "违规扣费申诉待审核", "有新的违规扣费申诉待管理员审核，申诉编号="+strconv.FormatUint(uint64(appeal.ID), 10))
	common.ApiSuccess(c, appeal)
}

func ListSelfViolationFeeRecords(c *gin.Context) {
	records, err := model.ListUserViolationFeeRecords(c.GetInt("id"), 100)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, records)
}

func ListAdminViolationFeeAppeals(c *gin.Context) {
	appeals, err := model.ListViolationFeeAppeals(c.Query("status"), 200)
	if err != nil {
		common.ApiErrorMsg(c, err.Error())
		return
	}
	common.ApiSuccess(c, appeals)
}

func ReviewAdminViolationFeeAppeal(c *gin.Context) {
	appealID, err := strconv.ParseUint(strings.TrimSpace(c.Param("id")), 10, 64)
	if err != nil || appealID == 0 {
		c.JSON(http.StatusBadRequest, gin.H{"success": false, "code": "VIOLATION_FEE_APPEAL_INVALID_ID", "message": "申诉编号无效"})
		return
	}
	var input violationFeeAppealReviewInput
	if c.Request.ContentLength != 0 {
		if err := c.ShouldBindJSON(&input); err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"success": false, "code": "VIOLATION_FEE_APPEAL_INVALID_REVIEW", "message": "审核意见格式无效"})
			return
		}
	}
	action := strings.ToLower(strings.TrimSpace(c.Param("action")))
	if action != "approve" && action != "reject" {
		c.JSON(http.StatusBadRequest, gin.H{"success": false, "code": "VIOLATION_FEE_APPEAL_INVALID_ACTION", "message": "审核动作无效"})
		return
	}
	approve := action == "approve"
	appeal, err := model.ReviewViolationFeeAppeal(c.GetInt("id"), uint(appealID), approve, input.Note)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	model.RecordLog(c.GetInt("id"), model.LogTypeSystem, "reviewed violation fee appeal "+strconv.FormatUint(appealID, 10))
	recordManageAuditFor(c, appeal.UserID, "security.violation_fee_appeal.review", map[string]interface{}{
		"appeal_id": appeal.ID,
		"approved":  approve,
	})
	common.ApiSuccess(c, appeal)
}
