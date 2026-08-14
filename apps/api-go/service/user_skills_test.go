package service

import (
	"fmt"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/model"
	"github.com/glebarez/sqlite"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"gorm.io/gorm"
)

func setupUserSkillsDB(t *testing.T) *gorm.DB {
	t.Helper()
	previous := model.DB
	dsn := fmt.Sprintf("file:user-skills-%p?mode=memory&cache=shared", t)
	db, err := gorm.Open(sqlite.Open(dsn), &gorm.Config{})
	require.NoError(t, err)
	model.DB = db
	require.NoError(t, db.AutoMigrate(&model.User{}, &model.AssistantMemory{}, &model.AssistantUserProfile{}))
	t.Cleanup(func() { model.DB = previous })
	return db
}

func createSkillUser(t *testing.T, db *gorm.DB, name string, role int) model.User {
	t.Helper()
	user := model.User{Username: name, Password: "password", Role: role, AffCode: name}
	require.NoError(t, db.Create(&user).Error)
	return user
}

func TestUserSkillsEnforcesStrictOwnerLattice(t *testing.T) {
	db := setupUserSkillsDB(t)
	first := createSkillUser(t, db, "skills-first", common.RoleCommonUser)
	second := createSkillUser(t, db, "skills-second", common.RoleCommonUser)
	admin := createSkillUser(t, db, "skills-admin", common.RoleAdminUser)
	peer := createSkillUser(t, db, "skills-peer", common.RoleAdminUser)
	root := createSkillUser(t, db, "skills-root", common.RoleRootUser)

	_, err := OpenSkills(first.Id, second.Id)
	assert.ErrorIs(t, err, model.ErrAssistantHistoryForbidden)
	_, err = OpenSkills(admin.Id, first.Id)
	require.NoError(t, err)
	_, err = OpenSkills(admin.Id, peer.Id)
	assert.ErrorIs(t, err, model.ErrAssistantHistoryForbidden)
	_, err = OpenSkills(admin.Id, root.Id)
	assert.ErrorIs(t, err, model.ErrAssistantHistoryForbidden)
}

func TestUserSkillsKeepsMemoriesPerUser(t *testing.T) {
	db := setupUserSkillsDB(t)
	first := createSkillUser(t, db, "skills-memory-first", common.RoleCommonUser)
	second := createSkillUser(t, db, "skills-memory-second", common.RoleCommonUser)
	firstSkills, err := OpenSkills(first.Id, first.Id)
	require.NoError(t, err)
	secondSkills, err := OpenSkills(second.Id, second.Id)
	require.NoError(t, err)

	_, err = firstSkills.Remember(MemoryDraft{Title: "Client", Content: "Uses Hermes.", Enabled: true})
	require.NoError(t, err)
	_, err = secondSkills.Remember(MemoryDraft{Title: "Client", Content: "Uses another client.", Enabled: true})
	require.NoError(t, err)

	firstRecall, err := firstSkills.Recall("Hermes", 4)
	require.NoError(t, err)
	require.Len(t, firstRecall, 1)
	secondRecall, err := secondSkills.Recall("Hermes", 4)
	require.NoError(t, err)
	assert.Empty(t, secondRecall)
}
