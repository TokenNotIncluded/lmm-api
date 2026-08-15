/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

package controller

import (
	"errors"
	"net/http"
	"strconv"
	"strings"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/service"
	"github.com/LIghtJUNction/api.lmm.best/setting/operation_setting"
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type publicRelayContributionInput struct {
	Name        string `json:"name"`
	BaseURL     string `json:"base_url"`
	Models      string `json:"models"`
	Description string `json:"description"`
}

type publicRelayReviewInput struct {
	Approve bool   `json:"approve"`
	Note    string `json:"note"`
}

type publicRelayReportInput struct {
	Reason string `json:"reason"`
}

type publicRelayReportReviewInput struct {
	Close bool   `json:"close"`
	Note  string `json:"note"`
}

type publicRelayWithdrawInput struct {
	Group string `json:"group"`
}

type publicRelayTipInput struct {
	AmountUSD float64 `json:"amount_usd"`
	Message   string  `json:"message"`
}

type publicRelayRatingInput struct {
	Rating  int    `json:"rating"`
	Comment string `json:"comment"`
}

type publicRelayRoutingInput struct {
	Disabled []int `json:"disabled_ids"`
	Ordered  []int `json:"order_ids"`
}

func publicRelayID(c *gin.Context) (int, bool) {
	id, err := strconv.Atoi(c.Param("id"))
	if err != nil || id <= 0 {
		c.AbortWithStatusJSON(http.StatusBadRequest, gin.H{"success": false, "code": "PUBLIC_RELAY_INVALID_ID", "message": "invalid public relay id"})
		return 0, false
	}
	return id, true
}

func publicRelayError(c *gin.Context, status int, code string, err error) {
	c.AbortWithStatusJSON(status, gin.H{"success": false, "code": code, "message": err.Error()})
}

func GetPublicRelayConfig(c *gin.Context) {
	common.ApiSuccess(c, gin.H{
		"group":                  operation_setting.GetPublicRelayGroup(),
		"minimum_withdrawal_usd": 10,
	})
}

func ListPublicRelayContributions(c *gin.Context) {
	limit, _ := strconv.Atoi(c.DefaultQuery("limit", "50"))
	items, err := model.ListApprovedPublicRelays(limit)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, gin.H{"items": items, "group": operation_setting.GetPublicRelayGroup()})
}

func CreatePublicRelayContribution(c *gin.Context) {
	var input publicRelayContributionInput
	if err := c.ShouldBindJSON(&input); err != nil {
		publicRelayError(c, http.StatusUnprocessableEntity, "PUBLIC_RELAY_INVALID_REQUEST", model.ErrPublicRelayInvalidInput)
		return
	}
	email, err := model.GetUserEmail(c.GetInt("id"))
	if err != nil || strings.TrimSpace(email) == "" {
		publicRelayError(c, http.StatusUnprocessableEntity, "PUBLIC_RELAY_EMAIL_REQUIRED", errors.New("a verified account email is required"))
		return
	}
	item, err := model.CreatePublicRelayContribution(c.GetInt("id"), email, input.Name, input.BaseURL, input.Models, input.Description)
	if err != nil {
		status, code := http.StatusUnprocessableEntity, "PUBLIC_RELAY_INVALID_REQUEST"
		if errors.Is(err, model.ErrPublicRelayInvalidURL) {
			code = "PUBLIC_RELAY_INVALID_URL"
		}
		publicRelayError(c, status, code, err)
		return
	}
	common.ApiSuccess(c, item)
}

func ListMyPublicRelayContributions(c *gin.Context) {
	limit, _ := strconv.Atoi(c.DefaultQuery("limit", "50"))
	items, err := model.ListUserPublicRelayContributions(c.GetInt("id"), limit)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, gin.H{"items": items, "group": operation_setting.GetPublicRelayGroup()})
}

func ListPublicRelayReviews(c *gin.Context) {
	id, ok := publicRelayID(c)
	if !ok {
		return
	}
	limit, _ := strconv.Atoi(c.DefaultQuery("limit", "20"))
	items, err := model.ListPublicRelayReviews(id, limit)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, gin.H{"items": items})
}

func RatePublicRelay(c *gin.Context) {
	id, ok := publicRelayID(c)
	if !ok {
		return
	}
	var input publicRelayRatingInput
	if err := c.ShouldBindJSON(&input); err != nil {
		publicRelayError(c, http.StatusUnprocessableEntity, "PUBLIC_RELAY_INVALID_REVIEW", model.ErrPublicRelayInvalidInput)
		return
	}
	if err := model.UpdatePublicRelayRating(id, c.GetInt("id"), input.Rating, input.Comment); err != nil {
		status := http.StatusUnprocessableEntity
		if errors.Is(err, model.ErrPublicRelayNotFound) {
			status = http.StatusNotFound
		}
		publicRelayError(c, status, "PUBLIC_RELAY_REVIEW_FAILED", err)
		return
	}
	common.ApiSuccess(c, nil)
}

func TipPublicRelay(c *gin.Context) {
	id, ok := publicRelayID(c)
	if !ok {
		return
	}
	var input publicRelayTipInput
	if err := c.ShouldBindJSON(&input); err != nil || input.AmountUSD <= 0 {
		publicRelayError(c, http.StatusUnprocessableEntity, "PUBLIC_RELAY_INVALID_TIP", model.ErrPublicRelayInvalidInput)
		return
	}
	quota := int64(common.QuotaFromFloat(input.AmountUSD * common.QuotaPerUnit))
	if err := model.TipPublicRelayContribution(id, c.GetInt("id"), quota, input.Message); err != nil {
		status := http.StatusUnprocessableEntity
		if errors.Is(err, model.ErrPublicRelayNotFound) {
			status = http.StatusNotFound
		}
		publicRelayError(c, status, "PUBLIC_RELAY_TIP_FAILED", err)
		return
	}
	common.ApiSuccess(c, gin.H{"amount_usd": input.AmountUSD})
}

func GetPublicRelayRouting(c *gin.Context) {
	items, group, err := model.ListPublicRelayRouting(c.GetInt("id"))
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, gin.H{"items": items, "group": group})
}

func UpdatePublicRelayRouting(c *gin.Context) {
	var input publicRelayRoutingInput
	if err := c.ShouldBindJSON(&input); err != nil {
		publicRelayError(c, http.StatusUnprocessableEntity, "PUBLIC_RELAY_INVALID_ROUTING", model.ErrPublicRelayInvalidInput)
		return
	}
	if err := model.UpdatePublicRelayRouting(c.GetInt("id"), operation_setting.GetPublicRelayGroup(), input.Disabled, input.Ordered); err != nil {
		publicRelayError(c, http.StatusUnprocessableEntity, "PUBLIC_RELAY_ROUTING_FAILED", err)
		return
	}
	common.ApiSuccess(c, nil)
}

func ListAdminPublicRelayContributions(c *gin.Context) {
	limit, _ := strconv.Atoi(c.DefaultQuery("limit", "100"))
	items, err := model.ListAdminPublicRelayContributions(c.Query("status"), limit)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, gin.H{"items": items, "group": operation_setting.GetPublicRelayGroup()})
}

func ReviewAdminPublicRelayContribution(c *gin.Context) {
	id, ok := publicRelayID(c)
	if !ok {
		return
	}
	var input publicRelayReviewInput
	if err := c.ShouldBindJSON(&input); err != nil {
		publicRelayError(c, http.StatusUnprocessableEntity, "PUBLIC_RELAY_INVALID_REQUEST", model.ErrPublicRelayInvalidInput)
		return
	}
	item, err := model.ReviewPublicRelayContribution(id, c.GetInt("id"), input.Approve, input.Note)
	if err != nil {
		status, code := http.StatusUnprocessableEntity, "PUBLIC_RELAY_REVIEW_FAILED"
		if errors.Is(err, model.ErrPublicRelayNotFound) {
			status, code = http.StatusNotFound, "PUBLIC_RELAY_NOT_FOUND"
		}
		if errors.Is(err, model.ErrPublicRelayAlreadyReviewed) {
			status, code = http.StatusConflict, "PUBLIC_RELAY_ALREADY_REVIEWED"
		}
		publicRelayError(c, status, code, err)
		return
	}
	common.ApiSuccess(c, item)
}

func ReportPublicRelayContribution(c *gin.Context) {
	id, ok := publicRelayID(c)
	if !ok {
		return
	}
	var input publicRelayReportInput
	if err := c.ShouldBindJSON(&input); err != nil {
		publicRelayError(c, http.StatusUnprocessableEntity, "PUBLIC_RELAY_INVALID_REPORT", model.ErrPublicRelayInvalidInput)
		return
	}
	report, err := model.CreatePublicRelayReport(id, c.GetInt("id"), input.Reason)
	if err != nil {
		status, code := http.StatusUnprocessableEntity, "PUBLIC_RELAY_REPORT_FAILED"
		if errors.Is(err, model.ErrPublicRelayNotFound) {
			status, code = http.StatusNotFound, "PUBLIC_RELAY_NOT_FOUND"
		}
		if errors.Is(err, gorm.ErrDuplicatedKey) {
			status, code = http.StatusConflict, "PUBLIC_RELAY_ALREADY_REPORTED"
		}
		publicRelayError(c, status, code, err)
		return
	}
	common.ApiSuccess(c, report)
}

func ListAdminPublicRelayReports(c *gin.Context) {
	limit, _ := strconv.Atoi(c.DefaultQuery("limit", "100"))
	items, err := model.ListAdminPublicRelayReports(c.Query("status"), limit)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, gin.H{"items": items})
}

func ReviewAdminPublicRelayReport(c *gin.Context) {
	id, ok := publicRelayID(c)
	if !ok {
		return
	}
	var input publicRelayReportReviewInput
	if err := c.ShouldBindJSON(&input); err != nil {
		publicRelayError(c, http.StatusUnprocessableEntity, "PUBLIC_RELAY_INVALID_REQUEST", model.ErrPublicRelayInvalidInput)
		return
	}
	if err := model.ReviewPublicRelayReport(id, c.GetInt("id"), input.Close, input.Note); err != nil {
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, nil)
}

func LinkAdminPublicRelayChannel(c *gin.Context) {
	id, ok := publicRelayID(c)
	if !ok {
		return
	}
	channelID, err := strconv.Atoi(c.Param("channel_id"))
	if err != nil || channelID <= 0 {
		publicRelayError(c, http.StatusBadRequest, "PUBLIC_RELAY_INVALID_CHANNEL", errors.New("invalid channel id"))
		return
	}
	if err := model.LinkPublicRelayChannel(id, channelID); err != nil {
		status := http.StatusUnprocessableEntity
		code := "PUBLIC_RELAY_LINK_FAILED"
		if errors.Is(err, model.ErrPublicRelayNotFound) {
			status, code = http.StatusNotFound, "PUBLIC_RELAY_NOT_FOUND"
		} else if errors.Is(err, model.ErrPublicRelayChannelLinked) {
			status, code = http.StatusConflict, "PUBLIC_RELAY_CHANNEL_LINKED"
		} else if errors.Is(err, model.ErrPublicRelayGroupMismatch) {
			code = "PUBLIC_RELAY_GROUP_MISMATCH"
		}
		publicRelayError(c, status, code, err)
		return
	}
	common.ApiSuccess(c, nil)
}

func WithdrawPublicRelayContributionReward(c *gin.Context) {
	id, ok := publicRelayID(c)
	if !ok {
		return
	}
	var input publicRelayWithdrawInput
	if err := c.ShouldBindJSON(&input); err != nil {
		publicRelayError(c, http.StatusUnprocessableEntity, "PUBLIC_RELAY_INVALID_REQUEST", model.ErrPublicRelayInvalidInput)
		return
	}
	group := strings.TrimSpace(input.Group)
	userGroup, _ := model.GetUserGroup(c.GetInt("id"), false)
	if group == "" || !service.IsUserSelectableGroup(userGroup, group) {
		publicRelayError(c, http.StatusUnprocessableEntity, "PUBLIC_RELAY_INVALID_GROUP", errors.New("the selected group is not available for this account"))
		return
	}
	amount, err := model.WithdrawPublicRelayTips(id, c.GetInt("id"), group)
	if err != nil {
		publicRelayError(c, http.StatusUnprocessableEntity, "PUBLIC_RELAY_WITHDRAW_FAILED", err)
		return
	}
	common.ApiSuccess(c, gin.H{"quota": amount, "group": group})
}
