package model

import (
	"errors"
	"os"
	"strings"
	"testing"

	"github.com/QuantumNous/new-api/common"
	"github.com/QuantumNous/new-api/constant"
	"github.com/glebarez/sqlite"
	"github.com/stretchr/testify/require"
	"gorm.io/gorm"
)

type migrationCapabilityModel struct {
	ID       uint   `gorm:"primaryKey"`
	Code     string `gorm:"unique"`
	Name     string `gorm:"index:idx_migration_capability_name,where:category IS NOT NULL,option:INCLUDE (category)"`
	Search   string `gorm:"index:idx_migration_capability_expression,expression:lower(name)"`
	Category string `gorm:"check:chk_migration_capability_category,category <> ''"`
	OwnerID  uint
	Owner    migrationCapabilityOwner `gorm:"constraint:OnDelete:CASCADE"`
}

type migrationCapabilityOwner struct {
	ID uint `gorm:"primaryKey"`
}

type fakeMigrationLock struct {
	identity   postgresDatabaseIdentity
	releases   int
	acquires   int
	err        error
	acquireErr error
	acquired   bool
	onAcquire  func() error
	onRelease  func()
}

func newFakeMigrationLock(database string) *fakeMigrationLock {
	return &fakeMigrationLock{identity: testPostgresDatabaseIdentity(database, int64(len(database)+100))}
}

func testPostgresDatabaseIdentity(database string, oid int64) postgresDatabaseIdentity {
	return postgresDatabaseIdentity{
		ServerAddress: "127.0.0.1",
		ServerPort:    5432,
		DatabaseName:  database,
		DatabaseOID:   oid,
	}
}

type fakeMigrationLockRegistry struct {
	identities    map[*gorm.DB]postgresDatabaseIdentity
	held          map[postgresDatabaseIdentity]bool
	acquireCounts map[postgresDatabaseIdentity]int
	releaseOrder  []string
}

func newFakeMigrationLockRegistry(identities map[*gorm.DB]postgresDatabaseIdentity) *fakeMigrationLockRegistry {
	return &fakeMigrationLockRegistry{
		identities:    identities,
		held:          make(map[postgresDatabaseIdentity]bool),
		acquireCounts: make(map[postgresDatabaseIdentity]int),
	}
}

func (registry *fakeMigrationLockRegistry) factory(db *gorm.DB) (migrationAdvisoryLock, error) {
	identity, ok := registry.identities[db]
	if !ok {
		return nil, errors.New("test database identity is missing")
	}
	lock := &fakeMigrationLock{identity: identity}
	lock.onAcquire = func() error {
		if registry.held[identity] {
			return errors.New("PostgreSQL migration advisory lock is held by another startup")
		}
		registry.held[identity] = true
		registry.acquireCounts[identity]++
		return nil
	}
	lock.onRelease = func() {
		delete(registry.held, identity)
		registry.releaseOrder = append(registry.releaseOrder, identity.DatabaseName)
	}
	return lock, nil
}

func TestMigrationAdvisoryLockKeyIsCrossLanguageContract(t *testing.T) {
	require.Equal(t, int64(0x4c4d4d4150490001), MigrationAdvisoryLockKey)
}

func TestVerifyPostgresRuntimeIdentityRequiresOneCanonicalApplicationSchema(t *testing.T) {
	valid := postgresRuntimeIdentity{
		DatabaseName: "lmm", SchemaName: "public", DatabaseUser: "lmm",
		ServerVersion: 180000, ConfiguredSearchPath: "public",
		EffectiveSearchPath: "pg_catalog,public",
	}
	require.NoError(t, verifyPostgresRuntimeIdentity(valid))
	quotedPublic := valid
	quotedPublic.ConfiguredSearchPath = `"public"`
	require.NoError(t, verifyPostgresRuntimeIdentity(quotedPublic))
	versioned := valid
	versioned.SchemaName = "lmm_prod_20260802"
	versioned.ConfiguredSearchPath = `"lmm_prod_20260802"`
	versioned.EffectiveSearchPath = "pg_catalog,lmm_prod_20260802"
	require.NoError(t, verifyPostgresRuntimeIdentity(versioned))

	tests := []struct {
		name      string
		mutate    func(postgresRuntimeIdentity) postgresRuntimeIdentity
		errorText string
	}{
		{
			name: "application schema and configured path disagree",
			mutate: func(identity postgresRuntimeIdentity) postgresRuntimeIdentity {
				identity.SchemaName = "lmm_prod_20260802"
				return identity
			},
			errorText: "configured PostgreSQL search_path",
		},
		{
			name: "reserved temporary schema",
			mutate: func(identity postgresRuntimeIdentity) postgresRuntimeIdentity {
				identity.SchemaName = "pg_temp_3"
				identity.ConfiguredSearchPath = "pg_temp_3"
				identity.EffectiveSearchPath = "pg_catalog,pg_temp_3"
				return identity
			},
			errorText: "not a safe unquoted identifier",
		},
		{
			name: "configured user schema token",
			mutate: func(identity postgresRuntimeIdentity) postgresRuntimeIdentity {
				identity.ConfiguredSearchPath = `"$user", public`
				return identity
			},
			errorText: "configured PostgreSQL search_path",
		},
		{
			name: "configured metadata schema",
			mutate: func(identity postgresRuntimeIdentity) postgresRuntimeIdentity {
				identity.ConfiguredSearchPath = "lmm_meta,public"
				// A missing lmm_meta schema is omitted from PostgreSQL's effective path.
				identity.EffectiveSearchPath = "pg_catalog,public"
				return identity
			},
			errorText: "configured PostgreSQL search_path",
		},
		{
			name: "configured path explicitly includes catalog",
			mutate: func(identity postgresRuntimeIdentity) postgresRuntimeIdentity {
				identity.ConfiguredSearchPath = "pg_catalog,public"
				return identity
			},
			errorText: "configured PostgreSQL search_path",
		},
		{
			name: "effective temporary schema",
			mutate: func(identity postgresRuntimeIdentity) postgresRuntimeIdentity {
				identity.EffectiveSearchPath = "pg_temp_3,pg_catalog,public"
				return identity
			},
			errorText: "effective PostgreSQL search_path",
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			require.ErrorContains(t, verifyPostgresRuntimeIdentity(test.mutate(valid)), test.errorText)
		})
	}
}

func (lock *fakeMigrationLock) Identity() postgresDatabaseIdentity {
	return lock.identity
}

func (lock *fakeMigrationLock) Acquire() error {
	lock.acquires++
	if lock.acquireErr != nil {
		return lock.acquireErr
	}
	if lock.onAcquire != nil {
		if err := lock.onAcquire(); err != nil {
			return err
		}
	}
	lock.acquired = true
	return nil
}

func (lock *fakeMigrationLock) Release() error {
	if lock.acquired {
		lock.releases++
		lock.acquired = false
		if lock.onRelease != nil {
			lock.onRelease()
		}
	}
	return lock.err
}

func TestDatabaseMigrationModeFromEnv(t *testing.T) {
	original, hadOriginal := os.LookupEnv(dbMigrationModeEnv)
	t.Cleanup(func() {
		if hadOriginal {
			require.NoError(t, os.Setenv(dbMigrationModeEnv, original))
		} else {
			require.NoError(t, os.Unsetenv(dbMigrationModeEnv))
		}
	})

	require.NoError(t, os.Unsetenv(dbMigrationModeEnv))
	mode, err := databaseMigrationModeFromEnv()
	require.NoError(t, err)
	require.Equal(t, DBMigrationModeApply, mode)

	for _, accepted := range []DBMigrationMode{DBMigrationModeApply, DBMigrationModeVerify} {
		t.Run(string(accepted), func(t *testing.T) {
			t.Setenv(dbMigrationModeEnv, string(accepted))
			mode, err := databaseMigrationModeFromEnv()
			require.NoError(t, err)
			require.Equal(t, accepted, mode)
		})
	}
	for _, rejected := range []string{"", " ", "VERIFY", "apply ", "disabled"} {
		t.Run("reject_"+rejected, func(t *testing.T) {
			t.Setenv(dbMigrationModeEnv, rejected)
			_, err := databaseMigrationModeFromEnv()
			require.ErrorContains(t, err, "must be exactly apply or verify")
		})
	}
}

func TestVerifyMigrationModeRejectsNonPostgresBeforeChoosingDatabase(t *testing.T) {
	t.Setenv(dbMigrationModeEnv, string(DBMigrationModeVerify))
	for _, dsn := range []string{"", "local", "user:pass@tcp(localhost:3306)/new_api"} {
		t.Run(dsn, func(t *testing.T) {
			t.Setenv("SQL_DSN", dsn)
			chooserCalled := false
			_, err := initDBWithMigrationSession(func(string, bool) (*gorm.DB, common.DatabaseType, error) {
				chooserCalled = true
				return nil, "", errors.New("chooser should not run")
			})
			require.ErrorContains(t, err, "requires SQL_DSN to use postgres:// or postgresql://")
			require.False(t, chooserCalled)
		})
	}
}

func TestVerifyMigrationModeAllowsPostgresToReachChooser(t *testing.T) {
	t.Setenv(dbMigrationModeEnv, string(DBMigrationModeVerify))
	t.Setenv("SQL_DSN", "postgresql://example.invalid/new_api")
	chooserErr := errors.New("chooser reached")
	chooserCalled := false
	_, err := initDBWithMigrationSession(func(envName string, isLog bool) (*gorm.DB, common.DatabaseType, error) {
		chooserCalled = true
		require.Equal(t, "SQL_DSN", envName)
		require.False(t, isLog)
		return nil, "", chooserErr
	})
	require.ErrorIs(t, err, chooserErr)
	require.True(t, chooserCalled)
}

func TestVerifyMigrationPhaseUsesOnlyVerifierUnderLock(t *testing.T) {
	lock := newFakeMigrationLock("primary")
	session := newStartupMigrationSession(DBMigrationModeVerify)
	session.lockFactory = func(*gorm.DB) (migrationAdvisoryLock, error) { return lock, nil }
	var statements []string
	err := session.runPrimaryPhase(nil, common.DatabaseTypePostgreSQL, func() error {
		statements = append(statements, "ALTER TABLE should_not_run")
		return nil
	}, func() error {
		statements = append(statements, "SELECT required_schema_capability")
		return nil
	})
	require.NoError(t, err)
	require.Equal(t, []string{"SELECT required_schema_capability"}, statements)
	require.NotContains(t, strings.Join(statements, " "), "ALTER")
	require.NoError(t, session.Close())
	require.Equal(t, 1, lock.releases)
}

func TestApplyMigrationPhaseRunsExistingPathsUnderLock(t *testing.T) {
	lock := newFakeMigrationLock("primary")
	session := newStartupMigrationSession(DBMigrationModeApply)
	session.lockFactory = func(*gorm.DB) (migrationAdvisoryLock, error) { return lock, nil }
	var trace []string
	require.NoError(t, session.runPrimaryPhase(nil, common.DatabaseTypePostgreSQL, func() error {
		trace = append(trace, "auto-migrate", "manual-ddl", "backfills", "retired-options")
		return nil
	}, func() error {
		return errors.New("verify path unexpectedly ran")
	}))
	require.Equal(t, []string{"auto-migrate", "manual-ddl", "backfills", "retired-options"}, trace)
	require.NoError(t, session.Close())
	require.Equal(t, 1, lock.releases)
}

func TestMigrationLockContentionFailsClosed(t *testing.T) {
	session := newStartupMigrationSession(DBMigrationModeVerify)
	session.lockFactory = func(*gorm.DB) (migrationAdvisoryLock, error) {
		return nil, errors.New("PostgreSQL migration advisory lock is held by another startup")
	}
	called := false
	err := session.runPrimaryPhase(nil, common.DatabaseTypePostgreSQL, func() error {
		called = true
		return nil
	}, func() error {
		called = true
		return nil
	})
	require.ErrorContains(t, err, "held by another startup")
	require.False(t, called)
}

func TestMigrationLockReleasedWhenPhaseFails(t *testing.T) {
	lock := newFakeMigrationLock("primary")
	session := newStartupMigrationSession(DBMigrationModeApply)
	session.lockFactory = func(*gorm.DB) (migrationAdvisoryLock, error) { return lock, nil }
	phaseErr := session.runPrimaryPhase(nil, common.DatabaseTypePostgreSQL, func() error {
		return errors.New("migration failed")
	}, func() error { return nil })
	require.EqualError(t, session.closeOnFailure(phaseErr), "migration failed")
	require.Equal(t, 1, lock.releases)
	require.NoError(t, session.Close())
	require.Equal(t, 1, lock.releases)
}

func TestDistinctPrimariesSharingLogDatabaseContendOnLogLock(t *testing.T) {
	primaryA, primaryB, logA, logB := new(gorm.DB), new(gorm.DB), new(gorm.DB), new(gorm.DB)
	identityA := testPostgresDatabaseIdentity("primary_a", 201)
	identityB := testPostgresDatabaseIdentity("primary_b", 202)
	logIdentity := testPostgresDatabaseIdentity("shared_log", 203)
	registry := newFakeMigrationLockRegistry(map[*gorm.DB]postgresDatabaseIdentity{
		primaryA: identityA,
		primaryB: identityB,
		logA:     logIdentity,
		logB:     logIdentity,
	})

	sessionA := newStartupMigrationSession(DBMigrationModeVerify)
	sessionA.lockFactory = registry.factory
	require.NoError(t, sessionA.runPrimaryPhase(primaryA, common.DatabaseTypePostgreSQL,
		func() error { return errors.New("apply must not run") }, func() error { return nil }))
	sessionB := newStartupMigrationSession(DBMigrationModeVerify)
	sessionB.lockFactory = registry.factory
	require.NoError(t, sessionB.runPrimaryPhase(primaryB, common.DatabaseTypePostgreSQL,
		func() error { return errors.New("apply must not run") }, func() error { return nil }))
	require.NoError(t, sessionA.runLogPhase(logA, common.DatabaseTypePostgreSQL,
		func() error { return errors.New("apply must not run") }, func() error { return nil }))

	logVerified := false
	err := sessionB.runLogPhase(logB, common.DatabaseTypePostgreSQL,
		func() error { return errors.New("apply must not run") }, func() error {
			logVerified = true
			return nil
		})
	require.ErrorContains(t, err, "held by another startup")
	require.False(t, logVerified)
	require.False(t, registry.held[identityB], "failed startup must release its primary lock")
	require.True(t, registry.held[identityA])
	require.True(t, registry.held[logIdentity])
	require.NoError(t, sessionA.Close())
	require.Empty(t, registry.held)
}

func TestSamePostgresDatabaseReusesPrimaryLockForLogMigration(t *testing.T) {
	primary, logDB := new(gorm.DB), new(gorm.DB)
	identity := testPostgresDatabaseIdentity("primary_and_log", 301)
	registry := newFakeMigrationLockRegistry(map[*gorm.DB]postgresDatabaseIdentity{
		primary: identity,
		logDB:   identity,
	})
	session := newStartupMigrationSession(DBMigrationModeApply)
	session.lockFactory = registry.factory
	require.NoError(t, session.runPrimaryPhase(primary, common.DatabaseTypePostgreSQL,
		func() error { return nil }, func() error { return errors.New("verify must not run") }))
	logApplied := false
	require.NoError(t, session.runLogPhase(logDB, common.DatabaseTypePostgreSQL, func() error {
		logApplied = true
		return nil
	}, func() error { return errors.New("verify must not run") }))
	require.True(t, logApplied)
	require.Equal(t, 1, registry.acquireCounts[identity])
	require.Len(t, session.locks, 1)
	require.NoError(t, session.Close())
	require.Empty(t, registry.held)
}

func TestLogMigrationFailureReleasesAllDatabaseLocksInReverseOrder(t *testing.T) {
	primary, logDB := new(gorm.DB), new(gorm.DB)
	primaryIdentity := testPostgresDatabaseIdentity("primary", 401)
	logIdentity := testPostgresDatabaseIdentity("log", 402)
	registry := newFakeMigrationLockRegistry(map[*gorm.DB]postgresDatabaseIdentity{
		primary: primaryIdentity,
		logDB:   logIdentity,
	})
	session := newStartupMigrationSession(DBMigrationModeApply)
	session.lockFactory = registry.factory
	require.NoError(t, session.runPrimaryPhase(primary, common.DatabaseTypePostgreSQL,
		func() error { return nil }, func() error { return errors.New("verify must not run") }))
	err := session.runLogPhase(logDB, common.DatabaseTypePostgreSQL,
		func() error { return errors.New("log migration failed") }, func() error { return nil })
	require.ErrorContains(t, err, "log migration failed")
	require.Empty(t, registry.held)
	require.Equal(t, []string{"log", "primary"}, registry.releaseOrder)
	require.NoError(t, session.Close())
}

func TestPostgresDatabaseIdentityAmbiguityFailsClosed(t *testing.T) {
	primary, logDB := new(gorm.DB), new(gorm.DB)
	registry := newFakeMigrationLockRegistry(map[*gorm.DB]postgresDatabaseIdentity{
		primary: testPostgresDatabaseIdentity("same_name", 501),
		logDB:   testPostgresDatabaseIdentity("same_name", 502),
	})
	session := newStartupMigrationSession(DBMigrationModeVerify)
	session.lockFactory = registry.factory
	require.NoError(t, session.runPrimaryPhase(primary, common.DatabaseTypePostgreSQL,
		func() error { return nil }, func() error { return nil }))
	err := session.runLogPhase(logDB, common.DatabaseTypePostgreSQL,
		func() error { return nil }, func() error { return nil })
	require.ErrorContains(t, err, "identity changed")
	require.Empty(t, registry.held)
}

func TestSQLiteMigrationModeRemainsExplicit(t *testing.T) {
	apply := newStartupMigrationSession(DBMigrationModeApply)
	apply.lockFactory = func(*gorm.DB) (migrationAdvisoryLock, error) {
		t.Fatal("SQLite apply must not acquire a PostgreSQL lock")
		return nil, nil
	}
	applied := false
	require.NoError(t, apply.runPrimaryPhase(nil, common.DatabaseTypeSQLite, func() error {
		applied = true
		return nil
	}, func() error { return nil }))
	require.True(t, applied)

	verify := newStartupMigrationSession(DBMigrationModeVerify)
	err := verify.runPrimaryPhase(nil, common.DatabaseTypeSQLite, func() error { return nil }, func() error { return nil })
	require.ErrorContains(t, err, "requires PostgreSQL")
}

func TestMigrationModeDoesNotBlockBusinessWrites(t *testing.T) {
	db, err := gorm.Open(sqlite.Open(":memory:"), &gorm.Config{})
	require.NoError(t, err)
	type businessRecord struct {
		ID   uint `gorm:"primaryKey"`
		Name string
	}
	require.NoError(t, db.AutoMigrate(&businessRecord{}))

	session := newStartupMigrationSession(DBMigrationModeVerify)
	require.False(t, session.Applies())
	require.NoError(t, db.Create(&businessRecord{Name: "allowed"}).Error)
	var count int64
	require.NoError(t, db.Model(&businessRecord{}).Count(&count).Error)
	require.EqualValues(t, 1, count)
}

func TestVerifySetupStateKeepsControllerClosedUntilReadOnlyVerificationSucceeds(t *testing.T) {
	originalDB := DB
	originalSetup := constant.IsSetup()
	t.Cleanup(func() {
		DB = originalDB
		constant.SetSetup(originalSetup)
	})

	db, err := gorm.Open(sqlite.Open(":memory:"), &gorm.Config{})
	require.NoError(t, err)
	require.NoError(t, db.AutoMigrate(&Setup{}))
	require.NoError(t, db.Exec("CREATE TABLE users (id integer primary key, role integer, deleted_at datetime)").Error)
	DB = db
	constant.SetSetup(false)

	err = CheckSetupForStartup(false)
	require.ErrorContains(t, err, "expected exactly one setup record")
	require.False(t, constant.IsSetup())

	require.NoError(t, db.Create(&Setup{Version: "test", InitializedAt: 1}).Error)
	err = CheckSetupForStartup(false)
	require.ErrorContains(t, err, "without a root user")
	require.False(t, constant.IsSetup())

	require.NoError(t, db.Exec("INSERT INTO users (id, role) VALUES (?, ?)", 1, common.RoleRootUser).Error)
	var setupRowsBefore, userRowsBefore int64
	require.NoError(t, db.Table("setups").Count(&setupRowsBefore).Error)
	require.NoError(t, db.Table("users").Count(&userRowsBefore).Error)
	require.NoError(t, CheckSetupForStartup(false))
	require.True(t, constant.IsSetup())
	var setupRowsAfter, userRowsAfter int64
	require.NoError(t, db.Table("setups").Count(&setupRowsAfter).Error)
	require.NoError(t, db.Table("users").Count(&userRowsAfter).Error)
	require.Equal(t, setupRowsBefore, setupRowsAfter)
	require.Equal(t, userRowsBefore, userRowsAfter)
}
