use std::{path::Path, time::Duration};

use log::debug;
use r2d2_sqlite::SqliteConnectionManager;

use crate::{
    database::{
        DatabasePool,
        builder::{DatabaseConnectionBuilder, DatabaseInFile, JournalMode},
    },
    errors::Result,
};

#[derive(Clone)]
pub enum InMemoryJournalMode {
    Memory,
    Off,
}

impl From<InMemoryJournalMode> for JournalMode {
    fn from(value: InMemoryJournalMode) -> Self {
        match value {
            InMemoryJournalMode::Memory => JournalMode::Memory,
            InMemoryJournalMode::Off => JournalMode::Off,
        }
    }
}

/// TypeState marker: no database location has been set yet.
#[derive(Clone)]
pub struct DatabaseInMemory;

impl DatabaseConnectionBuilder<DatabaseInMemory> {
    pub fn journal_mode(mut self, value: InMemoryJournalMode) -> Self {
        self.journal_mode = value.into();
        self
    }

    pub fn location<P>(self, value: P) -> DatabaseConnectionBuilder<DatabaseInFile>
    where
        P: AsRef<Path>,
    {
        DatabaseConnectionBuilder {
            location: DatabaseInFile(value.as_ref().display().to_string()),
            pool_size: 5,
            min_idle: Some(5),
            connection_timeout: self.connection_timeout,
            busy_timeout: Duration::from_secs(10),
            foreign_keys: self.foreign_keys,
            journal_mode: JournalMode::Delete,
            synchronous: self.synchronous,
            #[cfg(feature = "serialize")]
            contents: None,
        }
    }

    /// Starts the database as a writable copy of `bytes`, a database serialized with
    /// [`DatabasePool::serialize`], instead of empty. Building from a copy is much faster than
    /// creating the schema again, e.g. for a fresh database per test.
    #[cfg(feature = "serialize")]
    pub fn contents(mut self, bytes: impl Into<std::sync::Arc<[u8]>>) -> Self {
        self.contents = Some(bytes.into());
        self
    }

    pub fn build(self, name: &str) -> Result<DatabasePool> {
        let foreign_keys = self.foreign_keys;
        let journal_mode = self.journal_mode.clone();
        let synchronous = self.synchronous;
        let busy_timeout = self.busy_timeout;
        let pool_size = self.pool_size;
        let min_idle = self.min_idle;
        let label = ":memory:".to_string();

        debug!("Creating connection manager...");
        #[cfg(feature = "serialize")]
        let contents = self.contents.clone();
        let manager = SqliteConnectionManager::memory().with_init(move |conn| {
            // Before the pragmas, so that they apply to the copied database
            #[cfg(feature = "serialize")]
            if let Some(bytes) = &contents {
                conn.deserialize_read_exact(
                    crate::rusqlite::MAIN_DB,
                    &bytes[..],
                    bytes.len(),
                    false,
                )?;
            }
            Self::apply_pragmas(conn, foreign_keys, &journal_mode, synchronous, busy_timeout)
        });
        debug!("Connection manager created");

        self.build_with_manager(name, manager, pool_size, min_idle, label)
    }
}
