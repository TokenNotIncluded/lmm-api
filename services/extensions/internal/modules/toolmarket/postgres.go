// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"database/sql"
	"encoding/json"
	"io"
	"time"
)

// PostgresSchema belongs to the toolmarket module database. An installer applies
// it explicitly with its owner credential; ordinary startup never creates tables.
const PostgresSchema = `CREATE SCHEMA lmm_toolmarket;
CREATE TABLE lmm_toolmarket.snapshot (
 id smallint PRIMARY KEY CHECK (id = 1),
 version bigint NOT NULL CHECK (version > 0),
 ciphertext bytea NOT NULL CHECK (octet_length(ciphertext) <= 67108864)
);`

// InitPostgresStore creates only the initial encrypted record, after the
// installer has applied PostgresSchema. Existing data is never replaced.
func InitPostgresStore(ctx context.Context, db *sql.DB, key []byte) (*Store, error) {
	s, err := postgresStore(db, key)
	if err != nil {
		return nil, err
	}
	ciphertext, err := s.encryptSnapshot(emptyDB())
	if err != nil {
		return nil, err
	}
	if _, err = db.ExecContext(ctx, `INSERT INTO lmm_toolmarket.snapshot(id,version,ciphertext) VALUES(1,1,$1)`, ciphertext); err != nil {
		return nil, ErrConflict
	}
	return s, nil
}

// OpenPostgresStore verifies an existing encrypted snapshot. It does not own db;
// Close stops this module store without closing the host's connection pool.
func OpenPostgresStore(ctx context.Context, db *sql.DB, key []byte) (*Store, error) {
	s, err := postgresStore(db, key)
	if err != nil {
		return nil, err
	}
	var raw []byte
	if err = db.QueryRowContext(ctx, `SELECT ciphertext FROM lmm_toolmarket.snapshot WHERE id=1`).Scan(&raw); err != nil {
		return nil, ErrUnavailable
	}
	if _, err = s.decryptSnapshot(raw); err != nil {
		return nil, err
	}
	return s, nil
}

func postgresStore(db *sql.DB, key []byte) (*Store, error) {
	if db == nil || len(key) != 32 {
		return nil, ErrInvalid
	}
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, ErrInvalid
	}
	aead, err := cipher.NewGCM(block)
	if err != nil {
		return nil, ErrInvalid
	}
	return &Store{pg: db, aead: aead}, nil
}

func (s *Store) encryptSnapshot(d database) ([]byte, error) {
	plain, err := json.Marshal(d)
	if err != nil {
		return nil, ErrInvalid
	}
	if len(plain) > 63<<20 {
		return nil, ErrLimit
	}
	nonce := make([]byte, s.aead.NonceSize())
	if _, err := io.ReadFull(rand.Reader, nonce); err != nil {
		return nil, ErrUnavailable
	}
	return s.aead.Seal(nonce, nonce, plain, []byte("lmm-toolmarket-pg-v1")), nil
}

func (s *Store) decryptSnapshot(raw []byte) (database, error) {
	var d database
	if len(raw) < s.aead.NonceSize() || len(raw) > 64<<20 {
		return d, ErrInvalid
	}
	plain, err := s.aead.Open(nil, raw[:s.aead.NonceSize()], raw[s.aead.NonceSize():], []byte("lmm-toolmarket-pg-v1"))
	if err != nil {
		return d, ErrInvalid
	}
	if json.Unmarshal(plain, &d) != nil || d.Version != 1 || d.Servers == nil || d.Releases == nil || d.Installs == nil || d.Secrets == nil || d.States == nil || d.Executions == nil {
		return d, ErrInvalid
	}
	if w := d.Web; w != nil && (w.Services == nil || w.Versions == nil || w.Installations == nil || w.Favorites == nil) {
		return d, ErrInvalid
	}
	return d, nil
}

// Each transaction reads fresh state. FOR UPDATE serializes independent host
// processes so consent counters and durable dispatch claims cannot race. The
// callback never performs network I/O; the existing dispatch boundary remains
// committed before contacting an external tool. Commit errors stay uncertain.
func (s *Store) postgresTransaction(write bool, read func(database) error, update func(*database) error) error {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	tx, err := s.pg.BeginTx(ctx, &sql.TxOptions{ReadOnly: !write})
	if err != nil {
		return ErrUnavailable
	}
	defer tx.Rollback()
	query := `SELECT ciphertext FROM lmm_toolmarket.snapshot WHERE id=1`
	if write {
		query += ` FOR UPDATE`
	}
	var raw []byte
	if err = tx.QueryRowContext(ctx, query).Scan(&raw); err != nil {
		return ErrUnavailable
	}
	d, err := s.decryptSnapshot(raw)
	if err != nil {
		return err
	}
	if s.virtualBuiltins {
		overlayBuiltinCatalog(&d)
	}
	if !write {
		return read(d)
	}
	if err = update(&d); err != nil {
		return err
	}
	raw, err = s.encryptSnapshot(d)
	if err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, `UPDATE lmm_toolmarket.snapshot SET version=version+1,ciphertext=$1 WHERE id=1`, raw); err != nil {
		return ErrUnavailable
	}
	if err = tx.Commit(); err != nil {
		return ErrUnavailable
	}
	return nil
}
