// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package wallet

import (
	"context"
	"database/sql"
	"errors"
)

// Schema is installation-owner only. The runtime never initializes a database.
const Schema = `CREATE SCHEMA IF NOT EXISTS extension_wallet;
CREATE TABLE extension_wallet.transfer_links (
 owner_user bigint NOT NULL,
 request_key text NOT NULL CHECK(length(request_key) BETWEEN 16 AND 64),
 amount_credits bigint NOT NULL CHECK(amount_credits>0),
 token text NOT NULL UNIQUE CHECK(length(token)=64),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(owner_user,request_key)
);
REVOKE ALL ON SCHEMA extension_wallet FROM PUBLIC;
REVOKE ALL ON extension_wallet.transfer_links FROM PUBLIC;`

type PostgresRecords struct{ db *sql.DB }

func NewPostgresRecords(db *sql.DB) (*PostgresRecords, error) {
	if db == nil {
		return nil, ErrInvalid
	}
	return &PostgresRecords{db}, nil
}
func (p *PostgresRecords) PutIfAbsent(ctx context.Context, owner int64, in CreateInput, token string) (string, error) {
	if owner <= 0 || in.AmountCredits <= 0 || !validKey(in.RequestKey) || !validToken(token) {
		return "", ErrInvalid
	}
	tx, e := p.db.BeginTx(ctx, nil)
	if e != nil {
		return "", ErrUnknown
	}
	defer tx.Rollback()
	_, e = tx.ExecContext(ctx, `INSERT INTO extension_wallet.transfer_links(owner_user,request_key,amount_credits,token) VALUES($1,$2,$3,$4) ON CONFLICT(owner_user,request_key) DO NOTHING`, owner, in.RequestKey, in.AmountCredits, token)
	if e != nil {
		return "", ErrUnknown
	}
	var stored string
	var amount int64
	e = tx.QueryRowContext(ctx, `SELECT amount_credits,token FROM extension_wallet.transfer_links WHERE owner_user=$1 AND request_key=$2`, owner, in.RequestKey).Scan(&amount, &stored)
	if e != nil {
		return "", ErrUnknown
	}
	if amount != in.AmountCredits {
		return "", ErrConflict
	}
	if e = tx.Commit(); e != nil {
		return "", ErrUnknown
	}
	return stored, nil
}
func (p *PostgresRecords) Token(ctx context.Context, owner int64, key string) (string, error) {
	var token string
	e := p.db.QueryRowContext(ctx, `SELECT token FROM extension_wallet.transfer_links WHERE owner_user=$1 AND request_key=$2`, owner, key).Scan(&token)
	if errors.Is(e, sql.ErrNoRows) {
		return "", ErrNotFound
	}
	if e != nil {
		return "", ErrUnknown
	}
	return token, nil
}
