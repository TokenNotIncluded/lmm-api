package model

import (
	"fmt"
	"strings"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"github.com/LIghtJUNction/api.lmm.best/constant"
	"github.com/glebarez/sqlite"
	"github.com/stretchr/testify/require"
	"gorm.io/gorm"
)

func TestGetUnfinishedMidjourneyTasksIsBounded(t *testing.T) {
	previousDB := DB
	previousLogDB := LOG_DB
	previousLimit := constant.TaskQueryLimit
	previousMainDatabaseType := common.MainDatabaseType()
	previousLogDatabaseType := common.LogDatabaseType()
	t.Cleanup(func() {
		DB, LOG_DB = previousDB, previousLogDB
		constant.TaskQueryLimit = previousLimit
		common.SetDatabaseTypes(previousMainDatabaseType, previousLogDatabaseType)
	})

	dsn := fmt.Sprintf("file:%s?mode=memory&cache=shared", strings.ReplaceAll(t.Name(), "/", "_"))
	db, err := gorm.Open(sqlite.Open(dsn), &gorm.Config{})
	require.NoError(t, err)
	require.NoError(t, db.AutoMigrate(&Midjourney{}))
	DB, LOG_DB = db, db
	common.SetDatabaseTypes(common.DatabaseTypeSQLite, common.DatabaseTypeSQLite)
	constant.TaskQueryLimit = 2

	for i := 0; i < 5; i++ {
		require.NoError(t, db.Create(&Midjourney{
			MjId:       fmt.Sprintf("pending-%d", i),
			Progress:   "0%",
			SubmitTime: int64(i + 1),
		}).Error)
	}
	require.NoError(t, db.Create(&Midjourney{MjId: "finished", Progress: "100%"}).Error)

	tasks := GetUnfinishedMidjourneyTasks(constant.TaskQueryLimit)
	require.Len(t, tasks, 2)
	require.Equal(t, "pending-0", tasks[0].MjId)
	require.Equal(t, "pending-1", tasks[1].MjId)
}
