package middleware

import (
	"net/http"

	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/gin-gonic/gin"
)

func loadPaymentUser(c *gin.Context) (*model.User, bool) {
	user, err := model.GetUserById(c.GetInt("id"), false)
	if err != nil || user == nil {
		c.AbortWithStatusJSON(http.StatusInternalServerError, gin.H{
			"success": false,
			"message": "Unable to verify payment access.",
		})
		return nil, false
	}
	c.Set("payment_user", user)
	return user, true
}

// PaymentMethodAccessGate loads the audience profile used by per-method rules.
// Unlike PaymentAccessGate it does not apply the legacy all-payment block;
// quote and checkout handlers enforce the selected method's policy instead.
func PaymentMethodAccessGate() gin.HandlerFunc {
	return func(c *gin.Context) {
		if _, ok := loadPaymentUser(c); !ok {
			return
		}
		c.Next()
	}
}

func PaymentAccessGate() gin.HandlerFunc {
	return func(c *gin.Context) {
		user, ok := loadPaymentUser(c)
		if !ok {
			return
		}
		if model.IsPaymentRestricted(user) {
			c.AbortWithStatusJSON(http.StatusForbidden, gin.H{
				"success": false,
				"code":    "PAYMENT_UNAVAILABLE",
				"message": "Payment is unavailable for this account.",
			})
			return
		}
		c.Next()
	}
}
