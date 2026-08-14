package controller

import (
	"errors"
	"strconv"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/gin-gonic/gin"
)

type unifiedTodoReadRequest struct {
	Category string `json:"category"`
	IDs      []int  `json:"ids"`
	All      bool   `json:"all"`
}

// GetUnifiedTodos returns the current user's unified todo center. The model
// layer applies the ordinary-user/admin visibility boundary before serializing
// any request details.
func GetUnifiedTodos(c *gin.Context) {
	page, _ := strconv.Atoi(c.DefaultQuery("p", "1"))
	pageSize, _ := strconv.Atoi(c.DefaultQuery("page_size", c.DefaultQuery("size", "20")))
	result, err := model.GetUnifiedTodoCenter(
		c.GetInt("id"),
		c.GetInt("role"),
		c.Query("category"),
		page,
		pageSize,
	)
	if err != nil {
		if errors.Is(err, model.ErrUnifiedTodoCategory) {
			common.ApiErrorMsg(c, err.Error())
			return
		}
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, result)
}

// MarkUnifiedTodosRead acknowledges selected rows, or all rows in one
// category. category=all is accepted only with all=true.
func MarkUnifiedTodosRead(c *gin.Context) {
	var request unifiedTodoReadRequest
	if err := c.ShouldBindJSON(&request); err != nil {
		common.ApiErrorMsg(c, model.ErrUnifiedTodoReadBody.Error())
		return
	}
	marked, err := model.MarkUnifiedTodoReads(
		c.GetInt("id"),
		c.GetInt("role"),
		request.Category,
		request.IDs,
		request.All,
	)
	if err != nil {
		if errors.Is(err, model.ErrUnifiedTodoCategory) || errors.Is(err, model.ErrUnifiedTodoReadBody) {
			common.ApiErrorMsg(c, err.Error())
			return
		}
		common.ApiError(c, err)
		return
	}
	common.ApiSuccess(c, gin.H{
		"category": request.Category,
		"all":      request.All,
		"marked":   marked,
	})
}
