use std::path::PathBuf;
use std::{path::Path, time::Duration};

use log::debug;
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;

use crate::rusqlite::Transaction;

use crate::errors::{DatabaseError, Result};

type TxResult<R> = std::result::Result<R, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Copy)]
pub struct DdlVersion {
    pub version: u16,
    pub description: &'static str,
    pub sql: &'static str,
}

#[derive(Clone)]
pub enum JournalMode {
    Delete,
    Truncate,
    Persist,
    Memory,
    Wal,
    Off
}

#[derive(Clone)]
pub struct DatabaseConnectionBuilder {
    location: Option<PathBuf>,
    pool_size: u32,
    connection_timeout: u64,
    busy_timeout: u64,
    foreign_keys: bool,
    journal_mode: JournalMode
}

impl Default for DatabaseConnectionBuilder {
    fn default() -> Self {
        Self {
            location: None,
            pool_size: 8,
            busy_timeout: 5,
            connection_timeout: 5,
            foreign_keys: false,
            journal_mode: JournalMode::Delete
        }
    }
}

impl DatabaseConnectionBuilder {
    pub fn location<P>(mut self, value: P) -> Self where P: AsRef<Path>{
        self.location=Some(value.as_ref().to_path_buf());
        self
    }
    pub fn pool_size(mut self, value: u32) -> Self {
        self.pool_size=value;
        self
    }
    pub fn connection_timeout(mut self, value: u64) -> Self {
        self.connection_timeout=value;
        self
    }
    pub fn busy_timeout(mut self, value: u64) -> Self {
        self.busy_timeout=value;
        self
    }
    pub fn enable_foreign_keys(mut self) -> Self {
        self.foreign_keys = true;
        self
    } 
    pub fn journal_mode(mut self, value: JournalMode) -> Self {
        self.journal_mode = value;
        self
    }

    pub fn build(self) -> Result<DatabaseConnection> {
        if let Some(location) = self.location.clone() {
            let builder = self.clone();
            let manager = SqliteConnectionManager::file(&location).with_init(move |conn| {
                if builder.foreign_keys {
                    conn.execute_batch("PRAGMA foreign_keys = ON;")?
                }
                let mode_literal = match builder.journal_mode{
                    JournalMode::Delete => "DELETE",
                    JournalMode::Memory => "MEMORY",
                    JournalMode::Persist => "PERSIST",
                    JournalMode::Wal => "WAL",
                    JournalMode::Off => "OFF",
                    JournalMode::Truncate => "TRUNCATE"
                };
                conn.execute_batch(&format!("PRAGMA journal_mode =  {};", mode_literal))?;
                conn.busy_timeout(Duration::from_secs(builder.busy_timeout))
            });

            let pool = Pool::builder()
                .max_size(self.pool_size)
                .connection_timeout(Duration::from_secs(self.connection_timeout))
                .build(manager)
                .map_err(|e| DatabaseError::Connection(location.display().to_string(), e))?;

            Ok(DatabaseConnection {
                pool
            })
        }else{
            panic!("Database location not specified")
        }

    }
}

pub struct DatabaseConnection {
    pool: Pool<SqliteConnectionManager>,
}

impl DatabaseConnection {
    pub fn initialize<P: AsRef<Path>>(path: P) -> Result<Self> {
        let manager = SqliteConnectionManager::file(path.as_ref()).with_init(|conn| {
            conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = DELETE;")?;
            conn.busy_timeout(Duration::from_secs(5))
        });

        let pool = Pool::builder()
            .max_size(8)
            .connection_timeout(Duration::from_secs(5))
            .build(manager)
            .map_err(|e| DatabaseError::Connection(path.as_ref().display().to_string(), e))?;

        Ok(Self {
            pool
        })
    }

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
        self
            .connection()?
            .execute("VACUUM", [])
            .map_err(DatabaseError::SchemaCreation)?;

        Ok(())
    }

    fn connection(&self) -> Result<PooledConnection<SqliteConnectionManager>> {
        self.pool.get().map_err(DatabaseError::Pool)
    }
}
