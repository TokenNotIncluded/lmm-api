/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

package model

import (
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestPublicRelayTipsRemainPendingUntilWithdrawal(t *testing.T) {
	db := setupConsoleActivationTestDB(t)
	require.NoError(t, db.AutoMigrate(&PublicRelayContribution{}, &PublicRelayTip{}))

	owner := User{Username: "relay-owner", Password: "password", AffCode: "relay-owner-aff", Group: "default"}
	tipper := User{Username: "relay-tipper", Password: "password", AffCode: "relay-tipper-aff", Quota: int(common.QuotaPerUnit * 20), Group: "default"}
	require.NoError(t, db.Create(&owner).Error)
	require.NoError(t, db.Create(&tipper).Error)
	contribution := PublicRelayContribution{
		UserId: owner.Id, ContributorEmail: "owner@example.com", Name: "shared relay",
		BaseURL: "https://relay.example.com", Group: "FREE", Status: PublicRelayApproved,
		ChannelId: 1, CreatedAt: common.GetTimestamp(), UpdatedAt: common.GetTimestamp(),
	}
	require.NoError(t, db.Create(&contribution).Error)

	tipQuota := int64(common.QuotaPerUnit * 10)
	require.NoError(t, TipPublicRelayContribution(contribution.Id, tipper.Id, tipQuota, "thanks"))

	var afterTipOwner, afterTipper User
	require.NoError(t, db.First(&afterTipOwner, owner.Id).Error)
	require.NoError(t, db.First(&afterTipper, tipper.Id).Error)
	assert.Zero(t, afterTipOwner.Quota, "tips must not be spendable before withdrawal")
	assert.Equal(t, tipper.Quota-int(tipQuota), afterTipper.Quota)

	withdrawn, err := WithdrawPublicRelayTips(contribution.Id, owner.Id, "default")
	require.NoError(t, err)
	assert.Equal(t, tipQuota, withdrawn)
	require.NoError(t, db.First(&afterTipOwner, owner.Id).Error)
	assert.Equal(t, int(tipQuota), afterTipOwner.Quota)
}
