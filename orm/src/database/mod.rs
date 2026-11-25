use std::{
    future::Future,
    path::Path,
    pin::Pin,
    sync::{Arc, RwLock},
    time::Instant,
};

use futures_lite::future::block_on;
use log::debug;
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;

use crate::rusqlite::Transaction;

use crate::errors::{DatabaseError, Result};

type TxResult<R> = std::result::Result<R, Box<dyn std::error::Error + Send + Sync>>;

type PooledConn = PooledConnection<SqliteConnectionManager>;

/// Data migration hook of a [`DdlVersion`]: an async function, run in the same transaction
/// right after the migration's SQL. Write it as an `async fn` and box it, e.g.
/// `Some(|tx| Box::pin(my_migration(tx)))`.
pub type UpdateFn =
    for<'a, 'b> fn(&'a mut Transaction<'b>) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>>;

pub mod builder;

#[derive(Clone, Copy)]
pub struct DdlVersion {
    pub version: u16,
    pub description: &'static str,
    pub sql: &'static str,
    pub update_fn: Option<UpdateFn>,
}

/// Pool of SQLite connections. Cloning is cheap: clones share the same pool.
///
/// Every method is async: rusqlite is synchronous, so the work runs on the [`blocking`]
/// thread pool, which works under any async runtime (or none, e.g. a plain `block_on`).
///
/// The async closures passed to [`run_in_connection`](Self::run_in_connection) and
/// [`run_in_transaction`](Self::run_in_transaction) are driven to completion on that same
/// blocking thread, so a connection or transaction never leaves it and their futures need not
/// be `Send`. Inside them, only await the `_in` methods of the ORM (or other runtime-agnostic
/// futures): there is no runtime reactor on that thread, so e.g. tokio I/O or timers panic.
#[derive(Clone)]
pub struct DatabasePool {
    /// `None` once [`close`](Self::close)d, for every clone.
    pool: Arc<RwLock<Option<Pool<SqliteConnectionManager>>>>,
}

impl DatabasePool {
    pub(crate) fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self {
            pool: Arc::new(RwLock::new(Some(pool))),
        }
    }

    /// The underlying pool, or [`DatabaseError::ClosedConnection`] once closed.
    fn current(&self) -> Result<Pool<SqliteConnectionManager>> {
        self.pool
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or(DatabaseError::ClosedConnection())
    }

    /// Closes the pool for this and every clone of it, closing all its connections before
    /// returning. From then on, every method fails with [`DatabaseError::ClosedConnection`].
    /// Closing the last connection lets SQLite checkpoint the WAL and remove the `-wal` and `-shm`
    /// files, which a process exiting without dropping the pool (e.g. through
    /// `std::process::exit`) would leave behind.
    ///
    /// Connections in use are waited for, up to the pool's connection timeout; past it, those are
    /// closed once their current work ends. Closing twice does nothing.
    pub async fn close(&self) {
        let Some(pool) = self.pool.write().unwrap_or_else(|e| e.into_inner()).take() else {
            return;
        };
        debug!("Closing database pool...");
        blocking::unblock(move || {
            // The pool opens connections in the background to keep its idle ones, and one being
            // opened would outlive the pool. Holding as many as it can have rules that out
            let max_size = pool.max_size() as usize;
            let deadline = Instant::now() + pool.connection_timeout();
            let mut held = Vec::with_capacity(max_size);
            while held.len() < max_size {
                match pool.get_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(conn) => held.push(conn),
                    Err(e) => {
                        debug!("Closing the database pool with connections in use: {e}");
                        break;
                    }
                }
            }
            // The held connections are the last references to the pool: dropping them closes
            // every connection
            drop(pool);
            drop(held);
            debug!("Database pool closed");
        })
        .await
    }

    /// Whether [`close`](Self::close) was called on this pool or any clone of it.
    pub fn is_closed(&self) -> bool {
        self.pool
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_none()
    }

    /// Runs `f` inside a transaction on a pooled connection, committing it if `f` succeeds
    /// and rolling it back otherwise.
    pub async fn run_in_transaction<F, R>(&self, f: F) -> Result<R>
    where
        F: AsyncFnOnce(&mut Transaction) -> TxResult<R> + Send + 'static,
        R: Send + 'static,
    {
        let pool = self.current()?;
        blocking::unblock(move || transaction_blocking(&pool, |tx| block_on(f(tx)))).await
    }

    /// Runs `f` on a pooled connection, outside any transaction.
    pub async fn run_in_connection<F, R>(&self, f: F) -> Result<R>
    where
        F: AsyncFnOnce(&mut PooledConn) -> TxResult<R> + Send + 'static,
        R: Send + 'static,
    {
        let pool = self.current()?;
        blocking::unblock(move || connection_blocking(&pool, |conn| block_on(f(conn)))).await
    }

    /// [`run_in_connection`](Self::run_in_connection) for the builders' own synchronous
    /// statements, which need no executor on the blocking thread.
    pub(crate) async fn run_sync_in_connection<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&mut PooledConn) -> TxResult<R> + Send + 'static,
        R: Send + 'static,
    {
        let pool = self.current()?;
        blocking::unblock(move || connection_blocking(&pool, f)).await
    }

    /// [`run_in_transaction`](Self::run_in_transaction) for the builders' own synchronous
    /// statements, which need no executor on the blocking thread.
    pub(crate) async fn run_sync_in_transaction<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&mut Transaction) -> TxResult<R> + Send + 'static,
        R: Send + 'static,
    {
        let pool = self.current()?;
        blocking::unblock(move || transaction_blocking(&pool, f)).await
    }

    /// Rebuilds the database file, reclaiming the space of deleted data and defragmenting it. Runs
    /// on a pooled connection outside any transaction, as SQLite requires.
    pub async fn vacuum(&self) -> Result<()> {
        let pool = self.current()?;
        blocking::unblock(move || {
            let conn = get_connection(&pool)?;
            debug!("Running VACUUM...");
            conn.execute_batch("VACUUM;").map_err(DatabaseError::Vacuum)
        })
        .await
    }

    /// Writes a compacted copy of the database to `path` (`VACUUM INTO`), leaving the database
    /// itself untouched. The copy is a consistent snapshot even while other connections write,
    /// and holds every committed change, those still in the WAL file included. Fails if `path`
    /// already exists and is not empty.
    pub async fn vacuum_into(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let target = path
            .to_str()
            .ok_or_else(|| {
                DatabaseError::Vacuum(crate::rusqlite::Error::InvalidPath(path.to_path_buf()))
            })?
            .to_string();
        let pool = self.current()?;
        blocking::unblock(move || {
            let conn = get_connection(&pool)?;
            debug!("Running VACUUM INTO '{target}'...");
            conn.execute("VACUUM INTO ?1", [&target])
                .map(|_| ())
                .map_err(DatabaseError::Vacuum)
        })
        .await
    }

    pub async fn create_schema(&self, ddls: &[DdlVersion]) -> Result<()> {
        let pool = self.current()?;
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
                block_on(update_fn(tx)).map_err(|e| DatabaseError::SchemaCreation(Box::new(e)))?;
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
