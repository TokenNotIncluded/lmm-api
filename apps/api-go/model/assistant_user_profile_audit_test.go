package model

import (
	"context"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestPurgeAssistantUserProfileAuditsBeforeIsBoundedAndKeepsAdminAudit(t *testing.T) {
	setupConsoleActivationTestDB(t)
	require.NoError(t, DB.AutoMigrate(&AssistantUserProfileAudit{}))
	require.NoError(t, DB.Create(&[]AssistantUserProfileAudit{
		{UserId: 1, Source: AssistantProfileSourceAI, CreatedAt: 1},
		{UserId: 1, Source: AssistantProfileSourceAI, CreatedAt: 2},
		{UserId: 1, Source: AssistantProfileSourceAI, CreatedAt: 3},
		{UserId: 1, Source: AssistantProfileSourceAdmin, CreatedAt: 1},
		{UserId: 1, Source: AssistantProfileSourceAI, CreatedAt: 11},
	}).Error)

	deleted, err := PurgeAssistantUserProfileAuditsBefore(context.Background(), 10, 2)
	require.NoError(t, err)
	assert.EqualValues(t, 2, deleted)
	deleted, err = PurgeAssistantUserProfileAuditsBefore(context.Background(), 10, 2)
	require.NoError(t, err)
	assert.EqualValues(t, 1, deleted)
	deleted, err = PurgeAssistantUserProfileAuditsBefore(context.Background(), 10, 2)
	require.NoError(t, err)
	assert.Zero(t, deleted)

	var audits []AssistantUserProfileAudit
	require.NoError(t, DB.Order("id ASC").Find(&audits).Error)
	require.Len(t, audits, 2)
	assert.Equal(t, AssistantProfileSourceAdmin, audits[0].Source)
	assert.Equal(t, AssistantProfileSourceAI, audits[1].Source)
}
