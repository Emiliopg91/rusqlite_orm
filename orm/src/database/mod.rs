use log::debug;
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;

use crate::rusqlite::Transaction;

use crate::errors::{DatabaseError, Result};

type TxResult<R> = std::result::Result<R, Box<dyn std::error::Error + Send + Sync>>;

pub mod builder;

#[derive(Clone, Copy)]
pub struct DdlVersion {
    pub version: u16,
    pub description: &'static str,
    pub sql: &'static str,
    pub update_fn: Option<fn(&mut Transaction) -> Result<()>>,
}

pub struct DatabasePool {
    pool: Pool<SqliteConnectionManager>,
}

impl DatabasePool {
    pub fn run_in_transaction<F, R>(&self, mut f: F) -> Result<R>
    where
        F: FnMut(&mut Transaction) -> TxResult<R>,
    {
        let mut conn = self.connection()?;

        let mut tx = conn
            .transaction()
            .map_err(|e| DatabaseError::Transaction(Box::new(e)))?;

        let res = f(&mut tx).map_err(DatabaseError::Transaction)?;

        tx.commit()
            .map_err(|e| DatabaseError::Transaction(Box::new(e)))?;

        Ok(res)
    }

    pub fn run_in_connection<F, R>(&self, mut f: F) -> Result<R>
    where
        F: FnMut(&mut PooledConnection<SqliteConnectionManager>) -> TxResult<R>,
    {
        let mut conn = self.connection()?;

        let res = f(&mut conn).map_err(DatabaseError::RunningOnConnection)?;
        Ok(res)
    }

    pub fn create_schema(&self, ddls: &[DdlVersion]) -> Result<()> {
        let mut ddls: Vec<DdlVersion> = ddls.to_vec();
        ddls.sort_by_key(|ddl| ddl.version);
        let updated = self.run_in_transaction(|tx| {
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

        let conn = self.connection()?;

        debug!("Schema updated, running VACUUM to reclaim space...");
        conn.execute("VACUUM", [])
            .map_err(|e| DatabaseError::SchemaCreation(Box::new(e)))?;

        debug!("Running ANALYZE to refresh planner statistics...");
        conn.execute_batch("ANALYZE;")
            .map_err(|e| DatabaseError::SchemaCreation(Box::new(e)))?;

        Ok(())
    }

    fn connection(&self) -> Result<PooledConnection<SqliteConnectionManager>> {
        self.pool.get().map_err(DatabaseError::Pool)
    }
}
