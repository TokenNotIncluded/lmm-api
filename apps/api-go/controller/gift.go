package controller

import (
	"fmt"
	"net/http"
	"strconv"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/i18n"
	"github.com/LIghtJUNction/api.lmm.best/logger"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/gin-gonic/gin"
)

// GetAvailableGifts 用户获取当前可见礼包及领取状态（用于登录后礼包横幅）
func GetAvailableGifts(c *gin.Context) {
	userId := c.GetInt("id")
	gifts, err := model.GetAvailableGiftsForUser(userId)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	c.JSON(http.StatusOK, gin.H{
		"success": true,
		"data":    gifts,
	})
}

// ClaimGift 用户主动领取限时礼包。接口幂等：重复领取不会重复发放额度，
// 返回 200 与已有领取记录（data.already_claimed 为 true）。
func ClaimGift(c *gin.Context) {
	userId := c.GetInt("id")
	giftId, err := strconv.Atoi(c.Param("id"))
	if err != nil || giftId <= 0 {
		common.ApiErrorI18n(c, i18n.MsgInvalidParams)
		return
	}
	claim, alreadyClaimed, err := model.ClaimGift(userId, giftId)
	if err != nil {
		common.ApiErrorMsg(c, err.Error())
		return
	}
	if alreadyClaimed {
		c.JSON(http.StatusOK, gin.H{
			"success": true,
			"message": "已领取过该礼包",
			"data": gin.H{
				"claim":           claim,
				"already_claimed": true,
			},
		})
		return
	}
	model.RecordLog(userId, model.LogTypeTopup,
		fmt.Sprintf("领取补偿礼包，获得额度 %s", logger.LogQuota(claim.Quota)))
	c.JSON(http.StatusOK, gin.H{
		"success": true,
		"message": "领取成功",
		"data": gin.H{
			"claim":           claim,
			"already_claimed": false,
		},
	})
}

// AdminGetGifts 管理员获取全部礼包
func AdminGetGifts(c *gin.Context) {
	gifts, err := model.GetAllGifts()
	if err != nil {
		common.ApiError(c, err)
		return
	}
	c.JSON(http.StatusOK, gin.H{
		"success": true,
		"data":    gifts,
	})
}

type giftRequest struct {
	Title             string `json:"title"`
	Description       string `json:"description"`
	Quota             int    `json:"quota"`
	StartAt           int64  `json:"start_at"`
	EndAt             int64  `json:"end_at"`
	MinUsedQuota      int    `json:"min_used_quota"`
	MinAccountAgeDays int    `json:"min_account_age_days"`
	Enabled           *bool  `json:"enabled"`
}

func (r *giftRequest) validate() string {
	if r.Title == "" {
		return "礼包标题不能为空"
	}
	if r.Quota <= 0 {
		return "礼包额度必须为正数"
	}
	if r.EndAt <= r.StartAt {
		return "结束时间必须晚于开始时间"
	}
	if r.MinUsedQuota < 0 || r.MinAccountAgeDays < 0 {
		return "门槛参数不能为负数"
	}
	return ""
}

// AdminCreateGift 管理员创建礼包
func AdminCreateGift(c *gin.Context) {
	var req giftRequest
	if err := common.DecodeJson(c.Request.Body, &req); err != nil {
		common.ApiErrorI18n(c, i18n.MsgInvalidParams)
		return
	}
	if msg := req.validate(); msg != "" {
		common.ApiErrorMsg(c, msg)
		return
	}
	enabled := true
	if req.Enabled != nil {
		enabled = *req.Enabled
	}
	gift := &model.Gift{
		Title:             req.Title,
		Description:       req.Description,
		Quota:             req.Quota,
		StartAt:           req.StartAt,
		EndAt:             req.EndAt,
		MinUsedQuota:      req.MinUsedQuota,
		MinAccountAgeDays: req.MinAccountAgeDays,
		Enabled:           enabled,
	}
	if err := model.CreateGift(gift); err != nil {
		common.ApiError(c, err)
		return
	}
	c.JSON(http.StatusOK, gin.H{
		"success": true,
		"message": "创建成功",
		"data":    gift,
	})
}

// AdminUpdateGift 管理员编辑礼包
func AdminUpdateGift(c *gin.Context) {
	giftId, err := strconv.Atoi(c.Param("id"))
	if err != nil || giftId <= 0 {
		common.ApiErrorI18n(c, i18n.MsgInvalidParams)
		return
	}
	var req giftRequest
	if err := common.DecodeJson(c.Request.Body, &req); err != nil {
		common.ApiErrorI18n(c, i18n.MsgInvalidParams)
		return
	}
	if msg := req.validate(); msg != "" {
		common.ApiErrorMsg(c, msg)
		return
	}
	gift, err := model.GetGiftById(giftId)
	if err != nil {
		common.ApiErrorMsg(c, "礼包不存在")
		return
	}
	gift.Title = req.Title
	gift.Description = req.Description
	gift.Quota = req.Quota
	gift.StartAt = req.StartAt
	gift.EndAt = req.EndAt
	gift.MinUsedQuota = req.MinUsedQuota
	gift.MinAccountAgeDays = req.MinAccountAgeDays
	if req.Enabled != nil {
		gift.Enabled = *req.Enabled
	}
	if err := model.UpdateGift(gift); err != nil {
		common.ApiError(c, err)
		return
	}
	c.JSON(http.StatusOK, gin.H{
		"success": true,
		"message": "更新成功",
		"data":    gift,
	})
}

// AdminGetGiftClaims 管理员查看领取记录
func AdminGetGiftClaims(c *gin.Context) {
	giftId, _ := strconv.Atoi(c.DefaultQuery("gift_id", "0"))
	page, _ := strconv.Atoi(c.DefaultQuery("p", "1"))
	pageSize, _ := strconv.Atoi(c.DefaultQuery("page_size", "20"))
	if page < 1 {
		page = 1
	}
	if pageSize < 1 || pageSize > 100 {
		pageSize = 20
	}
	claims, total, err := model.GetGiftClaims(giftId, page, pageSize)
	if err != nil {
		common.ApiError(c, err)
		return
	}
	c.JSON(http.StatusOK, gin.H{
		"success": true,
		"data": gin.H{
			"items":     claims,
			"total":     total,
			"page":      page,
			"page_size": pageSize,
		},
	})
}
