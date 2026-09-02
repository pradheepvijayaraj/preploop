//! Schema creation and versioned migrations (#17).
//!
//! `run_migrations` applies each migration and its schema-version update in
//! one transaction so an interrupted upgrade can be retried safely.

use rusqlite::{params, Connection, Transaction, TransactionBehavior};

use super::helpers::table_has_column;
use super::DbResult;
use crate::backend::error::LoopError;
use crate::backend::error::ResultExt;

/// Current schema version.  Bump this whenever a new migration is added.
///
/// HOW TO ADD A MIGRATION:
/// 1. Increment `SCHEMA_VERSION` by 1.
/// 2. Add a `migrate_v<N>` function below the existing ones.
/// 3. Add the migration to `MIGRATIONS` in version order.
/// 4. Add a test in `tests.rs` to verify the migration on an in-memory DB.
#[derive(Clone, Copy)]
struct Migration {
    run: fn(&Connection) -> DbResult<()>,
    requires_foreign_keys_off: bool,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        run: migrate_v1,
        requires_foreign_keys_off: false,
    },
    Migration {
        run: migrate_v2,
        requires_foreign_keys_off: false,
    },
    Migration {
        run: migrate_v3,
        requires_foreign_keys_off: true,
    },
    Migration {
        run: migrate_v4_repair_foreign_keys,
        requires_foreign_keys_off: true,
    },
    Migration {
        run: migrate_v5_search_subsystem,
        requires_foreign_keys_off: false,
    },
    Migration {
        run: migrate_v6_mark_breakdown,
        requires_foreign_keys_off: false,
    },
];
const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

/// Run all outstanding migrations.
pub fn run_migrations(conn: &Connection) -> DbResult<()> {
    ensure_schema_version_table(conn)?;

    let current = get_schema_version(conn)?;
    if !(0..=SCHEMA_VERSION).contains(&current) {
        return Err(LoopError::internal(format!(
            "Unsupported database schema version {current}; latest is {SCHEMA_VERSION}"
        )));
    }
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let next_version = (index + 1) as i64;
        apply_migration(conn, *migration, next_version)?;

        if next_version == 3 {
            // VACUUM cannot run inside the atomic migration transaction. The
            // schema is already durable if this best-effort compaction fails.
            if let Err(error) = conn.execute_batch("VACUUM; PRAGMA optimize;") {
                log::warn!("Unable to compact the database after migration v3: {error}");
            }
        }
    }

    // Indices are created outside of versioned migrations because they're
    // idempotent (`IF NOT EXISTS`) and don't need version tracking.
    // Add new indices here freely without bumping SCHEMA_VERSION.
    conn.execute_batch(
        "
        CREATE INDEX IF NOT EXISTS idx_questions_bank_id ON questions(bank_id);
        CREATE INDEX IF NOT EXISTS idx_test_attempts_bank_id ON test_attempts(bank_id);
        ",
    )
    .stringify_err()?;

    Ok(())
}

fn apply_migration(conn: &Connection, migration: Migration, version: i64) -> DbResult<()> {
    let foreign_keys_were_enabled: bool = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .stringify_err()?;

    if migration.requires_foreign_keys_off && foreign_keys_were_enabled {
        // SQLite only accepts this pragma outside a transaction. V3 and V4
        // rebuild tables, so their foreign keys must be disabled before the
        // atomic transaction begins and restored after it ends.
        conn.pragma_update(None, "foreign_keys", "OFF")
            .stringify_err()?;
    }

    let migration_result = (|| -> DbResult<()> {
        let transaction =
            Transaction::new_unchecked(conn, TransactionBehavior::Immediate).stringify_err()?;
        (migration.run)(&transaction)?;
        set_schema_version(&transaction, version)?;
        transaction.commit().stringify_err()?;
        Ok(())
    })();

    let restore_result = if migration.requires_foreign_keys_off && foreign_keys_were_enabled {
        conn.pragma_update(None, "foreign_keys", "ON")
            .stringify_err()
    } else {
        Ok(())
    };

    migration_result?;
    restore_result
}

// ── Migrations ──────────────────────────────────────────────────────────

/// V1: initial schema (all core tables).
///
/// Uses `CREATE TABLE IF NOT EXISTS` so this migration is safe to re-run
/// on databases that were created before the migration system existed.
/// New databases also start here because `schema_version` defaults to 0.
fn migrate_v1(conn: &Connection) -> DbResult<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS question_banks (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            exam TEXT NOT NULL,
            metadata TEXT NOT NULL,
            total_questions INTEGER NOT NULL,
            difficulty TEXT NOT NULL,
            default_duration INTEGER NOT NULL,
            imported_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS categories (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            color TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS questions (
            id TEXT PRIMARY KEY,
            bank_id TEXT NOT NULL,
            type TEXT NOT NULL,
            question TEXT NOT NULL,
            options TEXT,
            correct_answers TEXT NOT NULL,
            explanation TEXT NOT NULL DEFAULT '',
            marks REAL NOT NULL,
            negative_marks REAL NOT NULL DEFAULT 0,
            negative_marks_unanswered REAL NOT NULL DEFAULT 0,
            time_estimate INTEGER,
            difficulty TEXT,
            tags TEXT,
            FOREIGN KEY (bank_id) REFERENCES question_banks(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS test_attempts (
            id TEXT PRIMARY KEY,
            bank_id TEXT NOT NULL,
            mode TEXT NOT NULL,
            status TEXT NOT NULL,
            duration INTEGER NOT NULL,
            time_remaining INTEGER NOT NULL,
            started_at INTEGER NOT NULL,
            completed_at INTEGER,
            score REAL,
            max_score REAL,
            FOREIGN KEY (bank_id) REFERENCES question_banks(id)
        );

        CREATE TABLE IF NOT EXISTS question_responses (
            id TEXT PRIMARY KEY,
            attempt_id TEXT NOT NULL,
            question_id TEXT NOT NULL,
            answer TEXT,
            is_correct INTEGER,
            is_flagged INTEGER NOT NULL DEFAULT 0,
            time_spent INTEGER,
            FOREIGN KEY (attempt_id) REFERENCES test_attempts(id) ON DELETE CASCADE,
            FOREIGN KEY (question_id) REFERENCES questions(id)
        );

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        ",
    )
    .stringify_err()
}

/// V2: add `category_id` column to `question_banks`.
fn migrate_v2(conn: &Connection) -> DbResult<()> {
    if table_has_column(conn, "question_banks", "category_id")? {
        return Ok(());
    }

    conn.execute("ALTER TABLE question_banks ADD COLUMN category_id TEXT", [])
        .stringify_err()?;

    Ok(())
}

/// V3: remove the abandoned category model and store only non-empty responses.
///
/// Correctness is derived when a submission is scored, so persisting it on
/// every response duplicated source-of-truth data. `time_spent` was never read.
/// The composite primary key also removes the synthetic response UUID and the
/// separate attempt index. Question options, answers and tags remain compact
/// JSON arrays because they are owned by one question and are always read with
/// it; splitting those arrays into child tables would add joins and row/index
/// overhead without reducing duplication.
fn migrate_v3(conn: &Connection) -> DbResult<()> {
    if !table_has_column(conn, "question_banks", "category_id")?
        && !table_has_column(conn, "question_responses", "id")?
    {
        return Ok(());
    }

    conn.execute_batch(
        "
        CREATE TABLE question_banks_v3 (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            exam TEXT NOT NULL,
            metadata TEXT NOT NULL,
            total_questions INTEGER NOT NULL,
            difficulty TEXT NOT NULL,
            default_duration INTEGER NOT NULL,
            imported_at INTEGER NOT NULL
        );

        CREATE TABLE questions_v3 (
            id TEXT PRIMARY KEY,
            bank_id TEXT NOT NULL,
            type TEXT NOT NULL,
            question TEXT NOT NULL,
            options TEXT,
            correct_answers TEXT NOT NULL,
            explanation TEXT NOT NULL DEFAULT '',
            marks REAL NOT NULL,
            negative_marks REAL NOT NULL DEFAULT 0,
            negative_marks_unanswered REAL NOT NULL DEFAULT 0,
            time_estimate INTEGER,
            difficulty TEXT,
            tags TEXT,
            FOREIGN KEY (bank_id) REFERENCES question_banks(id) ON DELETE CASCADE
        );

        CREATE TABLE test_attempts_v3 (
            id TEXT PRIMARY KEY,
            bank_id TEXT NOT NULL,
            mode TEXT NOT NULL,
            status TEXT NOT NULL,
            duration INTEGER NOT NULL,
            time_remaining INTEGER NOT NULL,
            started_at INTEGER NOT NULL,
            completed_at INTEGER,
            score REAL,
            max_score REAL,
            FOREIGN KEY (bank_id) REFERENCES question_banks(id) ON DELETE CASCADE
        );

        CREATE TABLE question_responses_v3 (
            attempt_id TEXT NOT NULL,
            question_id TEXT NOT NULL,
            answer TEXT,
            is_flagged INTEGER NOT NULL DEFAULT 0 CHECK (is_flagged IN (0, 1)),
            PRIMARY KEY (attempt_id, question_id),
            FOREIGN KEY (attempt_id) REFERENCES test_attempts_v3(id) ON DELETE CASCADE,
            FOREIGN KEY (question_id) REFERENCES questions_v3(id) ON DELETE CASCADE
        ) WITHOUT ROWID;

        INSERT INTO question_banks_v3
        SELECT id, name, exam, metadata, total_questions, difficulty,
               default_duration, imported_at
        FROM question_banks;

        INSERT INTO questions_v3
        SELECT id, bank_id, type, question, NULLIF(options, '[]'), correct_answers,
               explanation, marks, negative_marks, negative_marks_unanswered,
               NULLIF(time_estimate, 0), NULLIF(difficulty, ''), NULLIF(tags, '[]')
        FROM questions;

        INSERT INTO test_attempts_v3
        SELECT id, bank_id, mode, status, duration, time_remaining, started_at,
               completed_at, score, max_score
        FROM test_attempts;

        INSERT INTO question_responses_v3 (attempt_id, question_id, answer, is_flagged)
        SELECT attempt_id, question_id, answer, is_flagged
        FROM question_responses
        WHERE answer IS NOT NULL OR is_flagged != 0;

        DROP TABLE question_responses;
        DROP TABLE test_attempts;
        DROP TABLE questions;
        DROP TABLE question_banks;
        DROP TABLE IF EXISTS categories;

        ALTER TABLE question_banks_v3 RENAME TO question_banks;
        ALTER TABLE questions_v3 RENAME TO questions;
        ALTER TABLE test_attempts_v3 RENAME TO test_attempts;
        ALTER TABLE question_responses_v3 RENAME TO question_responses;
        ",
    )
    .stringify_err()?;

    Ok(())
}

/// V4: repair databases created by the original V3 migration.
///
/// SQLite retained references to the temporary `question_banks_v3` table when
/// that migration renamed it to `question_banks`. Rebuild the compact tables
/// with stable references to their final names. The migration is a no-op for
/// databases whose foreign-key metadata is already correct.
fn migrate_v4_repair_foreign_keys(conn: &Connection) -> DbResult<()> {
    let has_legacy_reference = ["questions", "test_attempts", "question_responses"]
        .iter()
        .try_fold(false, |found, table| -> DbResult<bool> {
            if found {
                return Ok(true);
            }

            let mut stmt = conn
                .prepare(&format!("PRAGMA foreign_key_list({table})"))
                .stringify_err()?;
            let mut rows = stmt.query([]).stringify_err()?;
            while let Some(row) = rows.next().stringify_err()? {
                let referenced_table: String = row.get(2).stringify_err()?;
                if referenced_table.ends_with("_v3") {
                    return Ok(true);
                }
            }
            Ok(false)
        })?;

    if !has_legacy_reference {
        return Ok(());
    }

    conn.execute_batch(
        "
        CREATE TABLE question_banks_repaired (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            exam TEXT NOT NULL,
            metadata TEXT NOT NULL,
            total_questions INTEGER NOT NULL,
            difficulty TEXT NOT NULL,
            default_duration INTEGER NOT NULL,
            imported_at INTEGER NOT NULL
        );

        CREATE TABLE questions_repaired (
            id TEXT PRIMARY KEY,
            bank_id TEXT NOT NULL,
            type TEXT NOT NULL,
            question TEXT NOT NULL,
            options TEXT,
            correct_answers TEXT NOT NULL,
            explanation TEXT NOT NULL DEFAULT '',
            marks REAL NOT NULL,
            negative_marks REAL NOT NULL DEFAULT 0,
            negative_marks_unanswered REAL NOT NULL DEFAULT 0,
            time_estimate INTEGER,
            difficulty TEXT,
            tags TEXT,
            FOREIGN KEY (bank_id) REFERENCES question_banks(id) ON DELETE CASCADE
        );

        CREATE TABLE test_attempts_repaired (
            id TEXT PRIMARY KEY,
            bank_id TEXT NOT NULL,
            mode TEXT NOT NULL,
            status TEXT NOT NULL,
            duration INTEGER NOT NULL,
            time_remaining INTEGER NOT NULL,
            started_at INTEGER NOT NULL,
            completed_at INTEGER,
            score REAL,
            max_score REAL,
            FOREIGN KEY (bank_id) REFERENCES question_banks(id) ON DELETE CASCADE
        );

        CREATE TABLE question_responses_repaired (
            attempt_id TEXT NOT NULL,
            question_id TEXT NOT NULL,
            answer TEXT,
            is_flagged INTEGER NOT NULL DEFAULT 0 CHECK (is_flagged IN (0, 1)),
            PRIMARY KEY (attempt_id, question_id),
            FOREIGN KEY (attempt_id) REFERENCES test_attempts(id) ON DELETE CASCADE,
            FOREIGN KEY (question_id) REFERENCES questions(id) ON DELETE CASCADE
        ) WITHOUT ROWID;

        INSERT INTO question_banks_repaired SELECT * FROM question_banks;
        INSERT INTO questions_repaired SELECT * FROM questions;
        INSERT INTO test_attempts_repaired SELECT * FROM test_attempts;
        INSERT INTO question_responses_repaired
            SELECT attempt_id, question_id, answer, is_flagged
            FROM question_responses;

        DROP TABLE question_responses;
        DROP TABLE test_attempts;
        DROP TABLE questions;
        DROP TABLE question_banks;

        ALTER TABLE question_banks_repaired RENAME TO question_banks;
        ALTER TABLE questions_repaired RENAME TO questions;
        ALTER TABLE test_attempts_repaired RENAME TO test_attempts;
        ALTER TABLE question_responses_repaired RENAME TO question_responses;
        ",
    )
    .stringify_err()?;

    Ok(())
}

/// V5: add search projection (`search_documents`), SQLite FTS5 table (`question_fts`),
/// persistent indexing queue (`search_index_jobs`), and taxonomy metadata (`question_taxonomy`).
fn migrate_v5_search_subsystem(conn: &Connection) -> DbResult<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS search_documents (
            search_id INTEGER PRIMARY KEY,
            question_id TEXT NOT NULL UNIQUE,
            question TEXT NOT NULL,
            options_text TEXT NOT NULL DEFAULT '',
            main_tag TEXT NOT NULL DEFAULT '',
            subtags_text TEXT NOT NULL DEFAULT '',
            bank_id TEXT NOT NULL,
            bank_name TEXT NOT NULL,
            year INTEGER,
            stage TEXT NOT NULL DEFAULT '',
            paper TEXT NOT NULL DEFAULT '',
            section TEXT NOT NULL DEFAULT '',
            content_fingerprint BLOB NOT NULL,
            FOREIGN KEY (question_id) REFERENCES questions(id) ON DELETE CASCADE
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS question_fts USING fts5(
            question,
            options_text,
            main_tag,
            subtags_text,
            content='search_documents',
            content_rowid='search_id',
            tokenize='porter unicode61',
            prefix='2 3'
        );

        -- FTS5 sync triggers for external content table
        CREATE TRIGGER IF NOT EXISTS search_docs_ai AFTER INSERT ON search_documents BEGIN
            INSERT INTO question_fts(rowid, question, options_text, main_tag, subtags_text)
            VALUES (new.search_id, new.question, new.options_text, new.main_tag, new.subtags_text);
        END;

        CREATE TRIGGER IF NOT EXISTS search_docs_ad AFTER DELETE ON search_documents BEGIN
            INSERT INTO question_fts(question_fts, rowid, question, options_text, main_tag, subtags_text)
            VALUES('delete', old.search_id, old.question, old.options_text, old.main_tag, old.subtags_text);
        END;

        CREATE TRIGGER IF NOT EXISTS search_docs_au AFTER UPDATE ON search_documents BEGIN
            INSERT INTO question_fts(question_fts, rowid, question, options_text, main_tag, subtags_text)
            VALUES('delete', old.search_id, old.question, old.options_text, old.main_tag, old.subtags_text);
            INSERT INTO question_fts(rowid, question, options_text, main_tag, subtags_text)
            VALUES (new.search_id, new.question, new.options_text, new.main_tag, new.subtags_text);
        END;

        CREATE TABLE IF NOT EXISTS search_index_jobs (
            question_id TEXT NOT NULL,
            operation TEXT NOT NULL CHECK (operation IN ('embed', 'delete', 'reclassify')),
            status TEXT NOT NULL DEFAULT 'pending'
                CHECK (status IN ('pending', 'running', 'failed', 'complete')),
            attempts INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (question_id, operation)
        );

        CREATE TABLE IF NOT EXISTS question_taxonomy (
            question_id TEXT PRIMARY KEY,
            main_tag TEXT NOT NULL,
            subtags_json TEXT NOT NULL DEFAULT '[]',
            taxonomy_source TEXT NOT NULL DEFAULT 'bundled'
                CHECK (taxonomy_source IN ('bundled', 'imported', 'automatic', 'manual')),
            taxonomy_version INTEGER NOT NULL DEFAULT 1,
            taxonomy_confidence REAL,
            FOREIGN KEY (question_id) REFERENCES questions(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_search_docs_bank_id ON search_documents(bank_id);
        CREATE INDEX IF NOT EXISTS idx_search_docs_section ON search_documents(section);
        CREATE INDEX IF NOT EXISTS idx_search_docs_stage ON search_documents(stage);
        CREATE INDEX IF NOT EXISTS idx_search_docs_paper ON search_documents(paper);
        CREATE INDEX IF NOT EXISTS idx_search_docs_year ON search_documents(year);
        CREATE INDEX IF NOT EXISTS idx_search_jobs_status ON search_index_jobs(status);
        ",
    )
    .stringify_err()?;

    // Populate search_documents and question_taxonomy from existing questions if any
    let mut stmt = conn
        .prepare(
            "SELECT q.id, q.bank_id, q.question, q.options, q.tags, b.name, b.metadata
             FROM questions q
             JOIN question_banks b ON b.id = q.bank_id",
        )
        .stringify_err()?;

    let rows = stmt
        .query_map([], |row| {
            let q_id: String = row.get(0)?;
            let bank_id: String = row.get(1)?;
            let question_text: String = row.get(2)?;
            let options_raw: Option<String> = row.get(3)?;
            let tags_raw: Option<String> = row.get(4)?;
            let bank_name: String = row.get(5)?;
            let bank_meta: String = row.get(6)?;
            Ok((
                q_id,
                bank_id,
                question_text,
                options_raw,
                tags_raw,
                bank_name,
                bank_meta,
            ))
        })
        .stringify_err()?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    for row_res in rows {
        let (q_id, bank_id, question_text, options_raw, tags_raw, bank_name, bank_meta) =
            row_res.stringify_err()?;

        // Parse options into text representation
        let mut opt_parts = Vec::new();
        if let Some(opts_json) = &options_raw {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(opts_json) {
                if let Some(arr) = val.as_array() {
                    for item in arr {
                        let id_str = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let text_str = item.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        if !id_str.is_empty() || !text_str.is_empty() {
                            opt_parts.push(format!("({id_str}) {text_str}"));
                        }
                    }
                }
            }
        }
        let options_text = opt_parts.join(" ");

        // Parse metadata (year, stage, paper, section)
        let mut year: Option<i64> = None;
        let mut stage = String::new();
        let mut paper = String::new();
        let mut section = String::new();
        if let Ok(meta_val) = serde_json::from_str::<serde_json::Value>(&bank_meta) {
            if let Some(y) = meta_val.get("year").and_then(|v| v.as_i64()) {
                year = Some(y);
            }
            if let Some(s) = meta_val.get("stage").and_then(|v| v.as_str()) {
                stage = s.to_string();
            }
            if let Some(p) = meta_val.get("paper").and_then(|v| v.as_str()) {
                paper = p.to_string();
            }
            if let Some(sec) = meta_val.get("section").and_then(|v| v.as_str()) {
                section = sec.to_string();
            }
        }

        // Parse tags
        let mut main_tag = String::new();
        let mut subtags = Vec::new();
        if let Some(tags_json) = &tags_raw {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(tags_json) {
                if let Some(arr) = val.as_array() {
                    for (i, t) in arr.iter().enumerate() {
                        if let Some(s) = t.as_str() {
                            if i == 0 {
                                main_tag = s.to_string();
                            } else {
                                subtags.push(s.to_string());
                            }
                        }
                    }
                }
            }
        }
        let subtags_text = subtags.join(" ");
        let subtags_json = serde_json::to_string(&subtags).unwrap_or_else(|_| "[]".to_string());

        // Canonical fingerprint (FNV-1a 64-bit)
        let canonical_str = if options_text.is_empty() {
            question_text.clone()
        } else {
            format!("{question_text}\n{options_text}")
        };
        let fp = crate::search::indexing::fingerprint::content_fingerprint(&canonical_str);
        let fp_bytes = fp.to_le_bytes();

        conn.execute(
            "INSERT OR IGNORE INTO search_documents (
                question_id, question, options_text, main_tag, subtags_text,
                bank_id, bank_name, year, stage, paper, section, content_fingerprint
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                q_id,
                question_text,
                options_text,
                main_tag,
                subtags_text,
                bank_id,
                bank_name,
                year,
                stage,
                paper,
                section,
                &fp_bytes[..],
            ],
        )
        .stringify_err()?;

        if !main_tag.is_empty() {
            conn.execute(
                "INSERT OR IGNORE INTO question_taxonomy (
                    question_id, main_tag, subtags_json, taxonomy_source, taxonomy_version
                ) VALUES (?1, ?2, ?3, 'bundled', 1)",
                params![q_id, main_tag, subtags_json],
            )
            .stringify_err()?;
        }

        conn.execute(
            "INSERT OR IGNORE INTO search_index_jobs (
                question_id, operation, status, created_at, updated_at
            ) VALUES (?1, 'embed', 'pending', ?2, ?2)",
            params![q_id, now],
        )
        .stringify_err()?;
    }

    Ok(())
}

/// V6: persist descriptive-paper mark allocations independently of question prose.
fn migrate_v6_mark_breakdown(conn: &Connection) -> DbResult<()> {
    if table_has_column(conn, "questions", "mark_breakdown")? {
        return Ok(());
    }

    conn.execute(
        "ALTER TABLE questions ADD COLUMN mark_breakdown TEXT NOT NULL DEFAULT '[]'",
        [],
    )
    .stringify_err()?;
    Ok(())
}

// ── Version helpers ─────────────────────────────────────────────────────

fn ensure_schema_version_table(conn: &Connection) -> DbResult<()> {
    let transaction =
        Transaction::new_unchecked(conn, TransactionBehavior::Immediate).stringify_err()?;

    transaction
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                version INTEGER NOT NULL
            );",
        )
        .stringify_err()?;

    if table_has_column(&transaction, "schema_version", "id")? {
        transaction
            .execute(
                "INSERT INTO schema_version (id, version) VALUES (1, 0)
                 ON CONFLICT(id) DO NOTHING",
                [],
            )
            .stringify_err()?;
    } else {
        // Upgrade the original unconstrained one-column table. MAX preserves
        // the furthest recorded migration if a damaged database contains more
        // than one legacy row, while the replacement enforces one row forever.
        transaction
            .execute_batch(
                "DROP TABLE IF EXISTS schema_version_singleton;
                 CREATE TABLE schema_version_singleton (
                    id INTEGER PRIMARY KEY CHECK (id = 1),
                    version INTEGER NOT NULL
                 );
                 INSERT INTO schema_version_singleton (id, version)
                 SELECT 1, COALESCE(MAX(version), 0) FROM schema_version;
                 DROP TABLE schema_version;
                 ALTER TABLE schema_version_singleton RENAME TO schema_version;",
            )
            .stringify_err()?;
    }

    transaction.commit().stringify_err()?;
    Ok(())
}

fn get_schema_version(conn: &Connection) -> DbResult<i64> {
    conn.query_row(
        "SELECT version FROM schema_version WHERE id = 1",
        [],
        |row| row.get(0),
    )
    .stringify_err()
}

fn set_schema_version(conn: &Connection, version: i64) -> DbResult<()> {
    conn.execute(
        "INSERT INTO schema_version (id, version) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET version = excluded.version",
        params![version],
    )
    .stringify_err()?;

    Ok(())
}
