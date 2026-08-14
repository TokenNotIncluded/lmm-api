/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

package controller

import (
	"strings"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/LIghtJUNction/api.lmm.best/setting/operation_setting"
	"github.com/shopspring/decimal"
)

// applyDiscountCodeQuote layers a discount code on top of the existing
// amount/group/payment pricing. The server re-runs this exact calculation at
// checkout, so the browser can only preview a price and never set the amount.
func applyDiscountCodeQuote(base decimal.Decimal, amount int64, rawCode string) (decimal.Decimal, *model.DiscountCode, error) {
	if strings.TrimSpace(rawCode) == "" {
		return base, nil, nil
	}
	code, err := model.ValidateDiscountCode(rawCode, amount, common.GetTimestamp())
	if err != nil {
		return decimal.Zero, nil, err
	}
	multiplier := decimal.NewFromInt(int64(100 - code.DiscountPercent)).Div(decimal.NewFromInt(100))
	return base.Mul(multiplier).Round(2), code, nil
}

func discountCodeID(code *model.DiscountCode) int {
	if code == nil {
		return 0
	}
	return code.Id
}

func discountPercent(code *model.DiscountCode) int {
	if code == nil {
		return 0
	}
	return code.DiscountPercent
}

func quoteTopUpWithDiscount(amount int64, group, paymentMethod, rawCode string) (decimal.Decimal, *model.DiscountCode, error) {
	base, err := quoteTopUp(amount, group, paymentMethod)
	if err != nil {
		return decimal.Zero, nil, err
	}
	return applyDiscountCodeQuote(base, amount, rawCode)
}

func quoteLegacyTopUpWithDiscount(amount int64, group, rawCode string) (decimal.Decimal, *model.DiscountCode, error) {
	base := quoteTopUpWithPricing(amount, group, decimal.NewFromFloat(operationPrice()), decimal.NewFromInt(1))
	return applyDiscountCodeQuote(base, amount, rawCode)
}

// operationPrice is kept as a tiny seam for tests and to avoid duplicating
// the legacy pricing expression at each payment adapter.
func operationPrice() float64 {
	return operation_setting.Price
}
