use std::time::Duration;

use log::debug;
use r2d2_sqlite::SqliteConnectionManager;

use crate::{
    database::{
        DatabasePool,
        builder::{DatabaseConnectionBuilder, DatabaseInFile, JournalMode},
    },
    errors::Result,
};

impl DatabaseConnectionBuilder<DatabaseInFile> {
    pub fn journal_mode(mut self, value: JournalMode) -> Self {
        self.journal_mode = value;
        self
    }

    pub fn min_idle(mut self, value: u32) -> Self {
        self.min_idle = Some(value);
        self
    }

    pub fn busy_timeout(mut self, value: Duration) -> Self {
        self.busy_timeout = value;
        self
    }

    pub fn build(self, name: &str) -> Result<DatabasePool> {
        let label = self.location.0.clone();
        let foreign_keys = self.foreign_keys;
        let journal_mode = self.journal_mode.clone();
        let busy_timeout = self.busy_timeout;
        let pool_size = self.pool_size;
        let min_idle = self.min_idle;

        debug!("Creating connection manager...");
        let manager = SqliteConnectionManager::file(&label).with_init(move |conn| {
            Self::apply_pragmas(conn, foreign_keys, &journal_mode, busy_timeout)
        });
        debug!("Connection manager created");

        self.build_with_manager(name, manager, pool_size, min_idle, label)
    }

    pub fn pool_size(mut self, value: u32) -> Self {
        self.pool_size = value;
        self
    }
}
