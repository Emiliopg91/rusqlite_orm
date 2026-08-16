pub mod errors;

use std::{path::Path, sync::OnceLock, time::Duration};

use log::debug;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

use crate::rusqlite::Transaction;

use self::errors::{DatabaseError, Result};

#[derive(Clone, Copy)]
pub struct DdlVersion {
    pub version: u16,
    pub description: &'static str,
    pub sql: &'static str,
}

pub struct Database {
    pool: Pool<SqliteConnectionManager>,
}

impl Database {
    pub fn open<P>(path: P) -> Result<Self>
    where
        P: AsRef<Path>,
    {
        let manager = SqliteConnectionManager::file(path.as_ref()).with_init(|conn| {
            conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
            conn.busy_timeout(Duration::from_secs(5))
        });

        let pool = Pool::builder()
            .max_size(8)
            .connection_timeout(Duration::from_secs(5))
            .build(manager)
            .map_err(|e| DatabaseError::Connection(path.as_ref().display().to_string(), e))?;

        Ok(Self { pool })
    }

    pub fn run<F, R>(&self, mut f: F) -> Result<R>
    where
        F: FnMut(
            &mut Transaction,
        ) -> std::result::Result<R, Box<dyn std::error::Error + Send + Sync>>,
    {
        let mut conn = self.pool.get().map_err(DatabaseError::Pool)?;

        let mut tx = conn
            .transaction()
            .map_err(|e| DatabaseError::Transaction(Box::new(e)))?;

        let res = f(&mut tx).map_err(DatabaseError::Transaction)?;

        tx.commit()
            .map_err(|e| DatabaseError::Transaction(Box::new(e)))?;

        Ok(res)
    }

    pub fn create_schema(&self, ddls: &[DdlVersion]) -> Result<()> {
        let updated = self.run(|tx| {
            let current_vers: u16 = tx
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .map_err(DatabaseError::SchemaCreation)?;

            let updates = ddls
                .iter()
                .filter(|update| update.version > current_vers)
                .cloned()
                .collect::<Vec<DdlVersion>>();

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

        if updated {
            debug!("Schema updated, running VACUUM to reclaim space...");

            let conn = self.pool.get().map_err(DatabaseError::Pool)?;
            conn.execute("VACUUM", [])
                .map_err(DatabaseError::SchemaCreation)
                .map(|_| ())
        } else {
            Ok(())
        }
    }
}

pub static DATABASE_INST: OnceLock<Database> = OnceLock::new();
