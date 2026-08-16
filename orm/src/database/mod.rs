pub mod errors;

use std::{path::Path, sync::OnceLock, time::Duration};

use log::debug;
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;

use crate::rusqlite::Transaction;

use self::errors::{DatabaseError, Result};

type TxResult<R> = std::result::Result<R, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Copy)]
pub struct DdlVersion {
    pub version: u16,
    pub description: &'static str,
    pub sql: &'static str,
}

pub struct Database {
    pool: Pool<SqliteConnectionManager>,
}

static DATABASE_INST: OnceLock<Database> = OnceLock::new();

impl Database {
    pub fn initialize<P: AsRef<Path>>(path: P) -> Result<()> {
        let manager = SqliteConnectionManager::file(path.as_ref()).with_init(|conn| {
            conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = DELETE;")?;
            conn.busy_timeout(Duration::from_secs(5))
        });

        let pool = Pool::builder()
            .max_size(8)
            .connection_timeout(Duration::from_secs(5))
            .build(manager)
            .map_err(|e| DatabaseError::Connection(path.as_ref().display().to_string(), e))?;

        DATABASE_INST
            .set(Self { pool })
            .map_err(|_| DatabaseError::AlreadyInitialized())
    }

    pub fn run_in_transaction<F, R>(f: F) -> Result<R>
    where
        F: FnOnce(&mut Transaction) -> TxResult<R>,
    {
        let mut conn = Self::instance()?.connection()?;

        let mut tx = conn
            .transaction()
            .map_err(|e| DatabaseError::Transaction(Box::new(e)))?;

        let res = f(&mut tx).map_err(DatabaseError::Transaction)?;

        tx.commit()
            .map_err(|e| DatabaseError::Transaction(Box::new(e)))?;

        Ok(res)
    }

    pub fn run<F, R>(f: F) -> Result<R>
    where
        F: FnOnce(&mut PooledConnection<SqliteConnectionManager>) -> TxResult<R>,
    {
        let mut conn = Self::instance()?.connection()?;

        let res = f(&mut conn).map_err(DatabaseError::RunningOnConnection)?;

        Ok(res)
    }

    pub fn create_schema(ddls: &[DdlVersion]) -> Result<()> {
        let updated = Self::run_in_transaction(|tx| {
            let current_version: u16 = tx
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .map_err(DatabaseError::SchemaCreation)?;

            let updates: Vec<DdlVersion> = ddls
                .iter()
                .filter(|update| update.version > current_version)
                .copied()
                .collect();

            for update in &updates {
                debug!(
                    "Applying database DDL patch v{}: {}",
                    update.version, update.description
                );
                tx.execute_batch(update.sql)
                    .map_err(DatabaseError::SchemaCreation)?;
            }

            if let Some(max_version) = updates.iter().map(|u| u.version).max() {
                tx.pragma_update(None, "user_version", max_version)
                    .map_err(DatabaseError::SchemaCreation)?;
                debug!("Database updated succesfully");
            }

            Ok(!updates.is_empty())
        })?;

        if !updated {
            return Ok(());
        }

        debug!("Schema updated, running VACUUM to reclaim space...");
        Self::instance()?
            .connection()?
            .execute("VACUUM", [])
            .map_err(DatabaseError::SchemaCreation)?;

        Ok(())
    }

    fn instance() -> Result<&'static Self> {
        DATABASE_INST.get().ok_or(DatabaseError::ClosedConnection())
    }

    fn connection(&self) -> Result<PooledConnection<SqliteConnectionManager>> {
        self.pool.get().map_err(DatabaseError::Pool)
    }
}
