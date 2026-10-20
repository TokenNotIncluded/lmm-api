package bounties

import (
	"context"
	"database/sql"
	_ "embed"
	"encoding/json"
	"errors"
	"time"
)

// Schema is explicit installer input. Opening the repository never executes DDL.
//
//go:embed schema.sql
var Schema string

type Postgres struct{ db *sql.DB }

func NewPostgres(ctx context.Context, db *sql.DB, expectedDatabase string) (*Postgres, error) {
	if db == nil || expectedDatabase == "" {
		return nil, ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	var name string
	var core bool
	var version int
	if e := db.QueryRowContext(ctx, `SELECT current_database(),EXISTS(SELECT 1 FROM pg_namespace WHERE nspname LIKE 'core\_%' ESCAPE '\')`).Scan(&name, &core); e != nil {
		return nil, ErrUnavailable
	}
	if name != expectedDatabase || core {
		return nil, ErrForbidden
	}
	if e := db.QueryRowContext(ctx, `SELECT version FROM lmm_bounties.contract WHERE singleton`).Scan(&version); e != nil || version != 1 {
		return nil, ErrUnavailable
	}
	return &Postgres{db}, nil
}
func (p *Postgres) Within(ctx context.Context, fn func(Tx) error) error {
	if fn == nil {
		return ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	tx, e := p.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if e != nil {
		return e
	}
	defer tx.Rollback()
	if _, e = tx.ExecContext(ctx, `SELECT pg_advisory_xact_lock(hashtextextended('lmm_bounties:records',0))`); e != nil {
		return e
	}
	if e = fn(&postgresTx{ctx, tx}); e != nil {
		return e
	}
	return tx.Commit()
}

type postgresTx struct {
	ctx context.Context
	tx  *sql.Tx
}

func bucket(k string) bool {
	return k == "projects" || k == "challenges" || k == "disputes" || k == "operations" || k == "notifications" || k == "ledger" || k == "web_operations"
}
func (t *postgresTx) Get(k, id string, out any) error {
	if !bucket(k) || !validID(id) {
		return ErrInvalid
	}
	var b []byte
	e := t.tx.QueryRowContext(t.ctx, `SELECT body FROM lmm_bounties.records WHERE bucket=$1 AND id=$2`, k, id).Scan(&b)
	if errors.Is(e, sql.ErrNoRows) {
		return ErrNotFound
	}
	if e != nil {
		return e
	}
	return json.Unmarshal(b, out)
}
func (t *postgresTx) Put(k, id string, v any) error {
	if !bucket(k) || !validID(id) {
		return ErrInvalid
	}
	b, e := json.Marshal(v)
	if e != nil || len(b) > 65536 {
		return ErrInvalid
	}
	_, e = t.tx.ExecContext(t.ctx, `INSERT INTO lmm_bounties.records(bucket,id,body) VALUES($1,$2,$3::jsonb) ON CONFLICT(bucket,id) DO UPDATE SET body=EXCLUDED.body`, k, id, string(b))
	return e
}

const recordWhere = `bucket=$1 AND ($2='' OR body->>'project_id'=$2) AND ($3='' OR body->>'owner_user_id'=$3) AND ($4='' OR body->>'participant_user_id'=$4) AND ($5='' OR body->>'owner_user_id'=$5 OR body->>'participant_user_id'=$5) AND (NOT $6 OR (body->>'status' IN ('published','paused') AND COALESCE((body->>'archived_at')::bigint,0)=0)) AND ($7='' OR body->>'status'=$7) AND ($8::int=-1 OR (COALESCE((body->>'archived_at')::bigint,0)>0)=($8::int=1)) AND (NOT $9 OR body->>'kind'='tip_transfer') AND ($10='' OR body->>'participant_user_id'=$10) AND COALESCE(body->>'status','')<>'deleted' AND ($11='' OR body->>'challenge_id'=$11)`

func queryArgs(k string, q Query) []any {
	archived := -1
	if q.Archived != nil {
		archived = 0
		if *q.Archived {
			archived = 1
		}
	}
	return []any{k, q.ProjectID, q.OwnerUserID, q.ParticipantUserID, q.PartyUserID, q.Public, q.Status, archived, q.TipsOnly, q.RecipientUserID, q.ChallengeID}
}
func (t *postgresTx) List(k string, q Query) ([]json.RawMessage, error) {
	if !bucket(k) || q.Limit < 1 || q.Limit > 100 || q.Offset < 0 {
		return nil, ErrInvalid
	}
	args := append(queryArgs(k, q), q.After, q.Limit, q.Offset)
	rows, e := t.tx.QueryContext(t.ctx, `SELECT body FROM lmm_bounties.records r WHERE `+recordWhere+` AND ($12='' OR CASE WHEN $6 THEN (COALESCE((body->>'published_at')::bigint,0),id)<(SELECT COALESCE((x.body->>'published_at')::bigint,0),x.id FROM lmm_bounties.records x WHERE x.bucket=$1 AND x.id=$12) WHEN $1='notifications' THEN (COALESCE((body->>'created_at')::bigint,0),id)<(SELECT COALESCE((x.body->>'created_at')::bigint,0),x.id FROM lmm_bounties.records x WHERE x.bucket=$1 AND x.id=$12) ELSE id>$12 END) ORDER BY CASE WHEN $6 THEN COALESCE((body->>'published_at')::bigint,0) END DESC,CASE WHEN $1='notifications' THEN COALESCE((body->>'created_at')::bigint,0) END DESC,CASE WHEN $6 OR $1='notifications' THEN id END DESC,CASE WHEN NOT $6 AND $1<>'notifications' THEN id END ASC LIMIT $13 OFFSET $14`, args...)
	if e != nil {
		return nil, e
	}
	defer rows.Close()
	out := []json.RawMessage{}
	for rows.Next() {
		var b []byte
		if e = rows.Scan(&b); e != nil {
			return nil, e
		}
		out = append(out, append(json.RawMessage(nil), b...))
	}
	return out, rows.Err()
}
func (t *postgresTx) Count(k string, q Query) (int64, error) {
	if !bucket(k) {
		return 0, ErrInvalid
	}
	var n int64
	e := t.tx.QueryRowContext(t.ctx, `SELECT count(*) FROM lmm_bounties.records WHERE `+recordWhere, queryArgs(k, q)...).Scan(&n)
	return n, e
}
func (t *postgresTx) Sum(k string, q Query) (int64, error) {
	if k != "notifications" {
		return 0, ErrInvalid
	}
	var n int64
	e := t.tx.QueryRowContext(t.ctx, `SELECT COALESCE(sum((body->>'quota')::bigint),0) FROM lmm_bounties.records WHERE `+recordWhere, queryArgs(k, q)...).Scan(&n)
	return n, e
}
func (t *postgresTx) MarkRead(recipient string, tipsOnly bool, now int64) error {
	_, e := t.tx.ExecContext(t.ctx, `UPDATE lmm_bounties.records SET body=jsonb_set(body,'{recipient_read_at}',to_jsonb($3::bigint)) WHERE bucket='notifications' AND body->>'participant_user_id'=$1 AND (NOT $2 OR body->>'kind'='tip_transfer') AND COALESCE((body->>'recipient_read_at')::bigint,0)=0`, recipient, tipsOnly, now)
	return e
}

func (t *postgresTx) All(k string, q Query) ([]json.RawMessage, error) {
	if !bucket(k) {
		return nil, ErrInvalid
	}
	rows, e := t.tx.QueryContext(t.ctx, `SELECT body FROM lmm_bounties.records WHERE `+recordWhere+` ORDER BY COALESCE((body->>'created_at')::bigint,0) DESC,id DESC`, queryArgs(k, q)...)
	if e != nil {
		return nil, e
	}
	defer rows.Close()
	out := []json.RawMessage{}
	for rows.Next() {
		var b []byte
		if e = rows.Scan(&b); e != nil {
			return nil, e
		}
		out = append(out, json.RawMessage(b))
	}
	return out, rows.Err()
}
func (t *postgresTx) Ratings(user string, owner bool) (int64, int64, error) {
	column := "owner_rating_score"
	identity := "participant_user_id"
	if owner {
		column = "contributor_rating_score"
		identity = "owner_user_id"
	}
	var sum, n int64
	e := t.tx.QueryRowContext(t.ctx, `SELECT COALESCE(sum((body->>'`+column+`')::bigint),0),count(*) FROM lmm_bounties.records WHERE bucket='challenges' AND body->>'`+identity+`'=$1 AND COALESCE((body->>'`+column+`')::bigint,0)>0 AND ($2 OR NOT COALESCE((body->>'owner_rating_overturned')::boolean,false))`, user, owner).Scan(&sum, &n)
	return sum, n, e
}

func (t *postgresTx) DistinctParticipants(project string) (int64, error) {
	var n int64
	e := t.tx.QueryRowContext(t.ctx, `SELECT count(DISTINCT body->>'participant_user_id') FROM lmm_bounties.records WHERE bucket='challenges' AND body->>'project_id'=$1`, project).Scan(&n)
	return n, e
}
func (t *postgresTx) Thanks(user string) (int64, error) {
	var n int64
	e := t.tx.QueryRowContext(t.ctx, `SELECT count(*) FROM lmm_bounties.records WHERE bucket='notifications' AND body->>'sender_user_id'=$1 AND body->>'kind'='tip_transfer' AND COALESCE((body->>'thanked_at')::bigint,0)>0`, user).Scan(&n)
	return n, e
}

func (t *postgresTx) Appeals(project string, now int64) (int64, int64, error) {
	var n, ends int64
	e := t.tx.QueryRowContext(t.ctx, `SELECT count(*),COALESCE(max((c.body->>'rejected_at')::bigint+604800),0) FROM lmm_bounties.records c WHERE c.bucket='challenges' AND c.body->>'project_id'=$1 AND c.body->>'status'='rejected' AND COALESCE((c.body->>'rejected_at')::bigint,0)>$2-604800 AND NOT EXISTS(SELECT 1 FROM lmm_bounties.records d WHERE d.bucket='disputes' AND d.body->>'challenge_id'=c.id AND d.body->>'status' IN ('resolved_paid','resolved_denied'))`, project, now).Scan(&n, &ends)
	return n, ends, e
}
