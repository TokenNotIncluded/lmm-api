// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"errors"
	"strings"
	"time"
	"unicode/utf8"
)

// The compatibility port supports the full original balance principal only.
// Partial, provider and guest refunds require separate frozen Core contracts.
type legacyRefundInput struct {
	RequestKey  string   `json:"request_key"`
	Reason      string   `json:"reason"`
	Mode        string   `json:"mode"`
	Quantity    int64    `json:"quantity,omitempty"`
	StockIDs    []string `json:"stock_ids,omitempty"`
	AmountQuota int64    `json:"amount_quota,omitempty"`
	AmountMinor int64    `json:"amount_minor,omitempty"`
}
type legacyRefundRecord struct {
	ID             string `json:"id"`
	RequestKey     string `json:"request_key"`
	InputHash      string `json:"input_hash"`
	Reason         string `json:"reason"`
	RequestedBy    int64  `json:"requested_by"`
	CreatedAt      int64  `json:"created_at"`
	Decision       string `json:"decision"`
	DecisionBy     int64  `json:"decision_by"`
	DecisionReason string `json:"decision_reason"`
	DecidedAt      int64  `json:"decided_at"`
	CompletedAt    int64  `json:"completed_at"`
}

func legacyRefundText(s string, required bool) bool {
	return utf8.ValidString(s) && len(s) <= 2000 && !strings.ContainsRune(s, 0) && (!required || strings.TrimSpace(s) != "")
}
func legacyRefundKey(s string) bool {
	if len(s) < 1 || len(s) > 64 {
		return false
	}
	for _, c := range s {
		if !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || c == '_' || c == '-') {
			return false
		}
	}
	return true
}

func legacyRefundDTO(o Order) map[string]any {
	r := o.LegacyRefund
	status := o.RefundState
	if o.State == RefundPending {
		status = "reconciliation_required"
	}
	return map[string]any{"id": r.ID, "order_id": o.ID, "request_key": r.RequestKey, "mode": "full", "quantity": o.Quantity,
		"stock_ids": []string{}, "amount_quota": o.Total.Amount, "amount_minor": int64(0), "currency": "", "reason": r.Reason,
		"status": status, "requested_by": r.RequestedBy, "requested_role": "buyer", "decision_by": r.DecisionBy,
		"decision_reason": r.DecisionReason, "retained_fee_quota": int64(0), "created_at": r.CreatedAt, "decided_at": r.DecidedAt, "completed_at": r.CompletedAt}
}

func legacyRefundView(o Order) map[string]any {
	refunded, reserved, remaining, quantity := int64(0), int64(0), o.Total.Amount, int64(0)
	refunds := []map[string]any{}
	if o.LegacyRefund != nil {
		refunds = append(refunds, legacyRefundDTO(o))
	}
	if o.State == Refunded {
		refunded, remaining, quantity = o.Total.Amount, 0, o.Quantity
	} else if o.RefundState == "requested" {
		reserved, remaining = o.Total.Amount, 0
	}
	return map[string]any{"supports_provider_sync": false, "provider_reconciliation_pending": false,
		"delivery_template": o.Legacy.DeliveryTemplate, "order_id": o.ID, "product_title": o.ProductTitle, "variant_name": o.Title,
		"payment_method": "balance", "currency": "", "principal_quota": o.Total.Amount, "refunded_quota": refunded,
		"reserved_quota": reserved, "remaining_quota": remaining, "quantity": o.Quantity, "refunded_quantity": quantity,
		"max_quantity": int64(0), "eligible_items": []any{}, "supports_quantity": false, "supports_amount": false,
		"native_basis_verified": false, "refunds": refunds}
}

func legacyRefundProof(tx Tx, shop string, o Order) error {
	if o.Legacy == nil || o.Legacy.PaymentMethod != "balance" || o.Total.Unit != "credits" || o.Total.Currency != "" ||
		o.Total.Amount <= 0 || o.Total.Amount > legacySafeInteger || o.Quantity <= 0 || o.Quantity > legacySafeInteger {
		return ErrUnavailable
	}
	var pay operation
	if err := tx.Get("money_operations", ID("store-money-v1", shop, o.ID, "pay"), &pay); err != nil {
		return err
	}
	if !pay.Done || pay.Before != AwaitingPayment || pay.Receipt.Outcome != "succeeded" || !validMoneyReceipt(pay.Command, pay.Receipt) ||
		pay.Command.Kind != "pay" || validateOrderCommand(shop, o, pay.Command) != nil || pay.Receipt.Reference != o.PaymentRef {
		return ErrConflict
	}
	return nil
}

func (s *Service) legacyRefund(ctx context.Context, token string, actor Account, row LocatedOrder, request LegacyRequest) (any, error) {
	o := row.Order
	if o.Buyer.Kind != "personal" || o.Seller.Kind != "personal" || actor != o.Buyer && actor != o.Seller {
		return nil, ErrForbidden
	}
	// Prove the original full principal against the same accepted payment used
	// by delivery. A public snapshot or an order's status string is not evidence.
	if err := s.repo.Within(ctx, row.ShopID, func(tx Tx) error {
		return legacyRefundProof(tx, row.ShopID, o)
	}); err != nil {
		return nil, err
	}
	if request.Action == "refunds" {
		if o.State == RefundPending {
			recovered, err := s.reconcileOrderMoney(ctx, row.ShopID, o.ID)
			if err != nil && !errors.Is(err, ErrPending) {
				return nil, err
			}
			if err == nil {
				o = recovered
			}
		}
		return legacyRefundView(o), nil
	}
	permission := Spend
	if request.Action == "decide_refund" {
		if actor != o.Seller {
			return nil, ErrForbidden
		}
		permission = Manage
	} else if actor != o.Buyer {
		return nil, ErrForbidden
	}
	if _, err := s.check(ctx, token, actor, permission); err != nil {
		return nil, err
	}
	approve := false
	err := s.repo.Within(ctx, row.ShopID, func(tx Tx) error {
		if err := tx.Get("orders", o.ID, &o); err != nil {
			return err
		}
		// The original parties and economics must not change between authorization
		// and mutation, including when another request completed the order.
		if validateOrderCommand(row.ShopID, o, orderMoneyCommand(row.ShopID, row.Order, "pay")) != nil {
			return ErrConflict
		}
		if err := legacyRefundProof(tx, row.ShopID, o); err != nil {
			return err
		}
		switch request.Action {
		case "request_refund":
			var in legacyRefundInput
			if legacyDecode(request.Body, &in) != nil || !legacyRefundKey(in.RequestKey) || !legacyRefundText(in.Reason, true) {
				return ErrInvalid
			}
			if in.Mode == "quantity" || in.Mode == "amount" {
				return ErrUnavailable
			}
			if in.Mode != "full" || in.Quantity != 0 || len(in.StockIDs) != 0 || in.AmountQuota != 0 || in.AmountMinor != 0 {
				return ErrInvalid
			}
			if o.LegacyRefund != nil {
				if o.LegacyRefund.InputHash != ID(in) {
					return ErrConflict
				}
				return nil
			}
			if o.RefundState != "" || o.State != Paid && o.State != Fulfilled && o.State != Settled {
				return ErrConflict
			}
			o.LegacyRefund = &legacyRefundRecord{ID: ID("store-refund-v1", row.ShopID, o.ID, in.RequestKey), RequestKey: in.RequestKey,
				InputHash: ID(in), Reason: in.Reason, RequestedBy: actor.ID, CreatedAt: time.Now().Unix()}
			o.RefundState, o.RefundReason = "requested", in.Reason
		case "decide_refund":
			var in struct {
				Decision string `json:"decision"`
				Reason   string `json:"reason"`
			}
			if legacyDecode(request.Body, &in) != nil || (in.Decision != "approve" && in.Decision != "reject") || !legacyRefundText(in.Reason, false) {
				return ErrInvalid
			}
			if o.LegacyRefund == nil || o.LegacyRefund.ID != request.ChildID {
				return ErrNotFound
			}
			r := o.LegacyRefund
			if r.Decision != "" {
				if r.Decision != in.Decision || r.DecisionReason != in.Reason || r.DecisionBy != actor.ID {
					return ErrConflict
				}
				approve = r.Decision == "approve"
				return nil
			}
			if o.RefundState != "requested" || o.State != Paid && o.State != Fulfilled && o.State != Settled {
				return ErrConflict
			}
			r.Decision, r.DecisionBy, r.DecisionReason, r.DecidedAt = in.Decision, actor.ID, in.Reason, time.Now().Unix()
			approve = in.Decision == "approve"
			if !approve {
				o.RefundState = "rejected"
			}
		default:
			return ErrInvalid
		}
		return tx.Put("orders", o.ID, o)
	})
	if err != nil {
		return nil, err
	}
	if approve {
		// Only this authenticated seller action may start the existing full refund.
		// Query first after a lost ACK. If still UNKNOWN, an explicit repeat of
		// the same seller approval may resume the SAME immutable operation: Core
		// rolls back liquidity failures, which otherwise have no receipt to read.
		// Core rechecks current authority and deduplicates the original key/hash.
		// GET/worker recovery never reaches this execution port.
		if o.State == RefundPending {
			var recovered Order
			recovered, err = s.reconcileOrderMoney(ctx, row.ShopID, o.ID)
			if err != nil && !errors.Is(err, ErrPending) {
				return nil, err
			}
			if err == nil {
				o = recovered
			}
		}
		if o.State != Refunded {
			o, err = s.ChangeOrder(ctx, token, OrderChange{ShopID: row.ShopID, OrderID: o.ID, Action: "refund"})
		}
		if err != nil {
			return nil, err
		}
	}
	return legacyRefundDTO(o), nil
}
