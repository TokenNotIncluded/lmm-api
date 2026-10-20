package toolmarket

import (
	"context"
	"database/sql"
)

// InitPostgresSnapshot is an explicit installer operation. Its INSERT belongs
// to the owner's schema transaction; existing records are never overwritten.
func InitPostgresSnapshot(ctx context.Context, db *sql.DB, tx *sql.Tx, key []byte) error {
	if tx == nil {
		return ErrInvalid
	}
	s, e := postgresStore(db, key)
	if e != nil {
		return e
	}
	ciphertext, e := s.encryptSnapshot(emptyDB())
	if e != nil {
		return e
	}
	_, e = tx.ExecContext(ctx, `INSERT INTO lmm_toolmarket.snapshot(id,version,ciphertext) VALUES(1,1,$1)`, ciphertext)
	return e
}
