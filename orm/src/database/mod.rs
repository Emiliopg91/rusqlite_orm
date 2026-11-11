use log::debug;
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;

use crate::rusqlite::Transaction;

use crate::errors::{DatabaseError, Result};

type TxResult<R> = std::result::Result<R, Box<dyn std::error::Error + Send + Sync>>;

type PooledConn = PooledConnection<SqliteConnectionManager>;

pub mod builder;

#[derive(Clone, Copy)]
pub struct DdlVersion {
    pub version: u16,
    pub description: &'static str,
    pub sql: &'static str,
    pub update_fn: Option<fn(&mut Transaction) -> Result<()>>,
}

/// Pool of SQLite connections. Cloning is cheap: clones share the same pool.
///
/// Every method that takes the pool is async: rusqlite is synchronous, so the work runs on
/// the [`blocking`] thread pool, which works under any async runtime (or none, e.g. a
/// plain `block_on`). The closures passed to [`run_in_connection`](Self::run_in_connection)
/// and [`run_in_transaction`](Self::run_in_transaction) stay synchronous and run there whole,
/// so a transaction never stays open across an `.await`.
#[derive(Clone)]
pub struct DatabasePool {
    pool: Pool<SqliteConnectionManager>,
}

impl DatabasePool {
    /// Runs `f` inside a transaction on a pooled connection, committing it if `f` succeeds
    /// and rolling it back otherwise.
    pub async fn run_in_transaction<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&mut Transaction) -> TxResult<R> + Send + 'static,
        R: Send + 'static,
    {
        let pool = self.pool.clone();
        blocking::unblock(move || transaction_blocking(&pool, f)).await
    }

    /// Runs `f` on a pooled connection, outside any transaction.
    pub async fn run_in_connection<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&mut PooledConn) -> TxResult<R> + Send + 'static,
        R: Send + 'static,
    {
        let pool = self.pool.clone();
        blocking::unblock(move || connection_blocking(&pool, f)).await
    }

    pub async fn create_schema(&self, ddls: &[DdlVersion]) -> Result<()> {
        let pool = self.pool.clone();
        let ddls = ddls.to_vec();
        blocking::unblock(move || create_schema_blocking(&pool, ddls)).await
    }
}

fn get_connection(pool: &Pool<SqliteConnectionManager>) -> Result<PooledConn> {
    pool.get().map_err(DatabaseError::Pool)
}

fn transaction_blocking<F, R>(pool: &Pool<SqliteConnectionManager>, f: F) -> Result<R>
where
    F: FnOnce(&mut Transaction) -> TxResult<R>,
{
    let mut conn = get_connection(pool)?;

    let mut tx = conn
        .transaction()
        .map_err(|e| DatabaseError::Transaction(Box::new(e)))?;

    let res = f(&mut tx).map_err(DatabaseError::Transaction)?;

    tx.commit()
        .map_err(|e| DatabaseError::Transaction(Box::new(e)))?;

    Ok(res)
}

fn connection_blocking<F, R>(pool: &Pool<SqliteConnectionManager>, f: F) -> Result<R>
where
    F: FnOnce(&mut PooledConn) -> TxResult<R>,
{
    let mut conn = get_connection(pool)?;

    f(&mut conn).map_err(DatabaseError::RunningOnConnection)
}

fn create_schema_blocking(
    pool: &Pool<SqliteConnectionManager>,
    mut ddls: Vec<DdlVersion>,
) -> Result<()> {
    ddls.sort_by_key(|ddl| ddl.version);
    let updated = transaction_blocking(pool, |tx| {
        let current_version: u16 = tx
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|e| DatabaseError::SchemaCreation(Box::new(e)))?;

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
                .map_err(|e| DatabaseError::SchemaCreation(Box::new(e)))?;
            if let Some(update_fn) = update.update_fn {
                debug!("Running update function...");
                update_fn(tx).map_err(|e| DatabaseError::SchemaCreation(Box::new(e)))?;
            }
        }

        if let Some(max_version) = updates.iter().map(|u| u.version).max() {
            tx.pragma_update(None, "user_version", max_version)
                .map_err(|e| DatabaseError::SchemaCreation(Box::new(e)))?;
            debug!("Database updated succesfully");
        }

        Ok(!updates.is_empty())
    })?;

    if !updated {
        return Ok(());
    }

    let conn = get_connection(pool)?;

    debug!("Schema updated, running VACUUM to reclaim space...");
    conn.execute("VACUUM", [])
        .map_err(|e| DatabaseError::SchemaCreation(Box::new(e)))?;

    debug!("Running ANALYZE to refresh planner statistics...");
    conn.execute_batch("ANALYZE;")
        .map_err(|e| DatabaseError::SchemaCreation(Box::new(e)))?;

    Ok(())
}
