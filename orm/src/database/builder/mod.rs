pub mod in_file;
pub mod in_memory;

use std::time::Duration;

use log::debug;
use r2d2::{HandleEvent, Pool};
use r2d2_sqlite::{SqliteConnectionManager, rusqlite::Connection};

use crate::{
    database::{DatabasePool, builder::in_memory::DatabaseInMemory},
    errors::{DatabaseError, Result},
};

#[derive(Clone)]
pub enum JournalMode {
    Delete,
    Truncate,
    Persist,
    Memory,
    Wal,
    Off,
}
pub fn literal_for(journal_mode: &JournalMode) -> String {
    match journal_mode {
        JournalMode::Delete => "DELETE",
        JournalMode::Memory => "MEMORY",
        JournalMode::Persist => "PERSIST",
        JournalMode::Wal => "WAL",
        JournalMode::Off => "OFF",
        JournalMode::Truncate => "TRUNCATE",
    }
    .to_string()
}

/// TypeState marker: a database location has been set.
#[derive(Clone)]
pub struct DatabaseInFile(String);

#[derive(Clone)]
pub struct DatabaseConnectionBuilder<L = DatabaseInMemory> {
    location: L,
    pool_size: u32,
    min_idle: Option<u32>,
    connection_timeout: Duration,
    busy_timeout: Duration,
    foreign_keys: bool,
    journal_mode: JournalMode,
}

impl Default for DatabaseConnectionBuilder<DatabaseInMemory> {
    fn default() -> Self {
        Self {
            location: DatabaseInMemory,
            pool_size: 1,
            min_idle: None,
            busy_timeout: Duration::from_secs(0),
            connection_timeout: Duration::from_secs(5),
            foreign_keys: false,
            journal_mode: JournalMode::Memory,
        }
    }
}

impl<L> DatabaseConnectionBuilder<L> {
    pub fn connection_timeout(mut self, value: Duration) -> Self {
        self.connection_timeout = value;
        self
    }
    pub fn enable_foreign_keys(mut self) -> Self {
        self.foreign_keys = true;
        self
    }

    fn apply_pragmas(
        conn: &Connection,
        foreign_keys: bool,
        journal_mode: &JournalMode,
        busy_timeout: Duration,
    ) -> crate::rusqlite::Result<()> {
        if foreign_keys {
            conn.execute_batch("PRAGMA foreign_keys = ON;")?
        }
        conn.execute_batch(&format!(
            "PRAGMA journal_mode =  {};",
            literal_for(journal_mode)
        ))?;

        let res = conn.busy_timeout(busy_timeout);

        res
    }

    fn build_with_manager(
        self,
        name: &str,
        manager: SqliteConnectionManager,
        pool_size: u32,
        min_idle: Option<u32>,
        label: String,
    ) -> Result<DatabasePool> {
        debug!(
            "Creating pool '{}' to '{}' with size: {} and connection timeout: {} ms ...",
            name,
            label,
            pool_size,
            self.connection_timeout.as_millis()
        );

        let pool = Pool::builder()
            .max_size(pool_size)
            .min_idle(min_idle)
            .connection_timeout(self.connection_timeout)
            .event_handler(Box::new(ConnectionLogger {
                conn_name: name.to_string(),
            }))
            .build(manager)
            .map_err(|e| DatabaseError::Connection(label, e))?;
        debug!("Pool '{}' created", name);

        Ok(DatabasePool { pool })
    }
}

#[derive(Debug)]
struct ConnectionLogger {
    conn_name: String,
}
impl HandleEvent for ConnectionLogger {
    fn handle_acquire(&self, event: r2d2::event::AcquireEvent) {
        debug!(
            "Acquired connection '{}::{}'...",
            self.conn_name,
            event.connection_id()
        );
    }
    fn handle_release(&self, event: r2d2::event::ReleaseEvent) {
        debug!(
            "Released connection '{}::{}'...",
            self.conn_name,
            event.connection_id()
        );
    }
}
