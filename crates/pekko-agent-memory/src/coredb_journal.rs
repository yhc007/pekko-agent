use async_trait::async_trait;
use pekko_persistence::{
    Journal, JournalResult, JournalError, PersistentRepr,
    SnapshotStore, SnapshotResult, SnapshotError, SnapshotMetadata,
};
use coredb::{CoreDB, QueryResult, CassandraValue};
use chrono::{DateTime, Utc};
use std::sync::Arc;
use tracing::{info, debug, warn};

/// CoreDB-backed Journal for event sourcing
///
/// Stores PersistentActor events using embedded coreDB.
/// Replaces PostgreSQL/Cassandra journal with zero external dependencies.
pub struct CoreDbJournal {
    db: Arc<CoreDB>,
    keyspace: String,
}

impl CoreDbJournal {
    /// Create a new CoreDB-backed journal
    pub async fn new(db: Arc<CoreDB>) -> Result<Self, JournalError> {
        let keyspace = "pekko_journal".to_string();

        Self::init_schema(&db, &keyspace).await?;

        info!(keyspace = %keyspace, "CoreDB journal initialized");

        Ok(Self { db, keyspace })
    }

    /// Initialize journal schema
    async fn init_schema(db: &CoreDB, keyspace: &str) -> Result<(), JournalError> {
        let cql = format!(
            "CREATE KEYSPACE IF NOT EXISTS {} WITH REPLICATION = {{'class': 'SimpleStrategy', 'replication_factor': 1}}",
            keyspace
        );
        db.execute_cql(&cql).await.map_err(|e| {
            JournalError::Connection(format!("Failed to create keyspace: {}", e))
        })?;

        // Journal events table
        // partition key: persistence_id, clustering key: sequence_nr
        let cql = format!(
            "CREATE TABLE IF NOT EXISTS {}.events (
                persistence_id TEXT,
                sequence_nr BIGINT,
                manifest TEXT,
                payload TEXT,
                timestamp TEXT,
                writer_uuid TEXT,
                tags TEXT,
                PRIMARY KEY (persistence_id, sequence_nr)
            )",
            keyspace
        );
        db.execute_cql(&cql).await.map_err(|e| {
            JournalError::Connection(format!("Failed to create events table: {}", e))
        })?;

        debug!("CoreDB journal schema initialized");
        Ok(())
    }

    /// Encode bytes to base64 for TEXT storage
    fn encode_payload(payload: &[u8]) -> String {
        use std::fmt::Write;
        let mut s = String::with_capacity(payload.len() * 2);
        for byte in payload {
            write!(s, "{:02x}", byte).unwrap();
        }
        s
    }

    /// Decode hex string back to bytes
    fn decode_payload(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .filter_map(|i| {
                if i + 2 <= hex.len() {
                    u8::from_str_radix(&hex[i..i + 2], 16).ok()
                } else {
                    None
                }
            })
            .collect()
    }

    /// Encode tags as comma-separated string
    fn encode_tags(tags: &[String]) -> String {
        tags.join(",")
    }

    /// Decode comma-separated tags
    fn decode_tags(tags_str: &str) -> Vec<String> {
        if tags_str.is_empty() {
            vec![]
        } else {
            tags_str.split(',').map(|s| s.to_string()).collect()
        }
    }
}

#[async_trait]
impl Journal for CoreDbJournal {
    async fn write_messages(&self, messages: Vec<PersistentRepr>) -> JournalResult<()> {
        for msg in &messages {
            let payload_hex = Self::encode_payload(&msg.payload);
            let tags_str = Self::encode_tags(&msg.tags);
            let escaped_manifest = msg.manifest.replace('\'', "''");
            let timestamp_str = msg.timestamp.to_rfc3339();

            let cql = format!(
                "INSERT INTO {}.events (persistence_id, sequence_nr, manifest, payload, timestamp, writer_uuid, tags) VALUES ('{}', {}, '{}', '{}', '{}', '{}', '{}')",
                self.keyspace,
                msg.persistence_id,
                msg.sequence_nr,
                escaped_manifest,
                payload_hex,
                timestamp_str,
                msg.writer_uuid,
                tags_str
            );

            self.db.execute_cql(&cql).await.map_err(|e| {
                JournalError::Write(format!("Failed to write event: {}", e))
            })?;
        }

        debug!(count = messages.len(), "Events written to CoreDB journal");
        Ok(())
    }

    async fn replay_messages(
        &self,
        persistence_id: &str,
        from_sequence_nr: u64,
        to_sequence_nr: u64,
        max_messages: Option<u64>,
    ) -> JournalResult<Vec<PersistentRepr>> {
        let limit_clause = match max_messages {
            Some(limit) => format!(" LIMIT {}", limit),
            None => String::new(),
        };

        let cql = format!(
            "SELECT persistence_id, sequence_nr, manifest, payload, timestamp, writer_uuid, tags FROM {}.events WHERE persistence_id = '{}'{}",
            self.keyspace, persistence_id, limit_clause
        );

        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => {
                let messages: Vec<PersistentRepr> = rows
                    .iter()
                    .filter_map(|row| {
                        let seq_nr = match row.columns.get("sequence_nr") {
                            Some(CassandraValue::BigInt(n)) => *n as u64,
                            _ => return None,
                        };

                        // Filter by sequence number range
                        if seq_nr < from_sequence_nr || seq_nr > to_sequence_nr {
                            return None;
                        }

                        let persistence_id = match row.columns.get("persistence_id") {
                            Some(CassandraValue::Text(s)) => s.clone(),
                            _ => return None,
                        };
                        let manifest = match row.columns.get("manifest") {
                            Some(CassandraValue::Text(s)) => s.clone(),
                            _ => String::new(),
                        };
                        let payload_hex = match row.columns.get("payload") {
                            Some(CassandraValue::Text(s)) => s.clone(),
                            _ => return None,
                        };
                        let timestamp_str = match row.columns.get("timestamp") {
                            Some(CassandraValue::Text(s)) => s.clone(),
                            _ => return None,
                        };
                        let writer_uuid_str = match row.columns.get("writer_uuid") {
                            Some(CassandraValue::Text(s)) => s.clone(),
                            _ => uuid::Uuid::new_v4().to_string(),
                        };
                        let tags_str = match row.columns.get("tags") {
                            Some(CassandraValue::Text(s)) => s.clone(),
                            _ => String::new(),
                        };

                        let payload = Self::decode_payload(&payload_hex);
                        let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp_str)
                            .map(|dt| dt.with_timezone(&Utc))
                            .unwrap_or_else(|_| Utc::now());
                        let writer_uuid = uuid::Uuid::parse_str(&writer_uuid_str)
                            .unwrap_or_else(|_| uuid::Uuid::new_v4());
                        let tags = Self::decode_tags(&tags_str);

                        Some(PersistentRepr {
                            persistence_id,
                            sequence_nr: seq_nr,
                            manifest,
                            payload,
                            timestamp,
                            writer_uuid,
                            tags,
                        })
                    })
                    .collect();

                debug!(
                    persistence_id = %persistence_id,
                    from = from_sequence_nr,
                    to = to_sequence_nr,
                    replayed = messages.len(),
                    "Events replayed from CoreDB journal"
                );

                Ok(messages)
            }
            Ok(_) => Ok(vec![]),
            Err(e) => Err(JournalError::Read(format!("Failed to replay: {}", e))),
        }
    }

    async fn highest_sequence_nr(&self, persistence_id: &str) -> JournalResult<u64> {
        let cql = format!(
            "SELECT MAX(sequence_nr) FROM {}.events WHERE persistence_id = '{}'",
            self.keyspace, persistence_id
        );

        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => {
                if let Some(row) = rows.first() {
                    for (_, val) in &row.columns {
                        if let CassandraValue::BigInt(n) = val {
                            return Ok(*n as u64);
                        }
                    }
                }
                Ok(0)
            }
            Ok(_) => Ok(0),
            Err(_) => Ok(0),
        }
    }

    async fn delete_messages_to(
        &self,
        persistence_id: &str,
        to_sequence_nr: u64,
    ) -> JournalResult<()> {
        // Fetch sequence numbers to delete
        let cql = format!(
            "SELECT sequence_nr FROM {}.events WHERE persistence_id = '{}'",
            self.keyspace, persistence_id
        );

        if let Ok(QueryResult::Rows(rows)) = self.db.execute_cql(&cql).await {
            for row in &rows {
                if let Some(CassandraValue::BigInt(seq)) = row.columns.get("sequence_nr") {
                    if (*seq as u64) <= to_sequence_nr {
                        let delete_cql = format!(
                            "DELETE FROM {}.events WHERE persistence_id = '{}' AND sequence_nr = {}",
                            self.keyspace, persistence_id, seq
                        );
                        let _ = self.db.execute_cql(&delete_cql).await;
                    }
                }
            }
        }

        info!(persistence_id = %persistence_id, to = to_sequence_nr, "Deleted journal events from CoreDB");
        Ok(())
    }

    async fn persistence_ids(&self) -> JournalResult<Vec<String>> {
        let cql = format!(
            "SELECT DISTINCT persistence_id FROM {}.events",
            self.keyspace
        );

        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => {
                let ids: Vec<String> = rows
                    .iter()
                    .filter_map(|row| {
                        match row.columns.get("persistence_id") {
                            Some(CassandraValue::Text(s)) => Some(s.clone()),
                            _ => None,
                        }
                    })
                    .collect();
                Ok(ids)
            }
            Ok(_) => Ok(vec![]),
            Err(e) => Err(JournalError::Read(format!("Failed to list IDs: {}", e))),
        }
    }
}

/// CoreDB-backed Snapshot Store
pub struct CoreDbSnapshotStore {
    db: Arc<CoreDB>,
    keyspace: String,
}

impl CoreDbSnapshotStore {
    /// Create a new CoreDB-backed snapshot store
    pub async fn new(db: Arc<CoreDB>) -> Result<Self, SnapshotError> {
        let keyspace = "pekko_journal".to_string();

        Self::init_schema(&db, &keyspace).await?;

        info!(keyspace = %keyspace, "CoreDB snapshot store initialized");

        Ok(Self { db, keyspace })
    }

    /// Initialize snapshot table schema
    async fn init_schema(db: &CoreDB, keyspace: &str) -> Result<(), SnapshotError> {
        let cql = format!(
            "CREATE KEYSPACE IF NOT EXISTS {} WITH REPLICATION = {{'class': 'SimpleStrategy', 'replication_factor': 1}}",
            keyspace
        );
        db.execute_cql(&cql).await.map_err(|e| {
            SnapshotError::Save(format!("Failed to create keyspace: {}", e))
        })?;

        let cql = format!(
            "CREATE TABLE IF NOT EXISTS {}.snapshots (
                persistence_id TEXT,
                sequence_nr BIGINT,
                timestamp TEXT,
                snapshot_data TEXT,
                PRIMARY KEY (persistence_id, sequence_nr)
            )",
            keyspace
        );
        db.execute_cql(&cql).await.map_err(|e| {
            SnapshotError::Save(format!("Failed to create snapshots table: {}", e))
        })?;

        debug!("CoreDB snapshot schema initialized");
        Ok(())
    }
}

#[async_trait]
impl SnapshotStore for CoreDbSnapshotStore {
    async fn save_snapshot(
        &self,
        metadata: &SnapshotMetadata,
        snapshot_data: Vec<u8>,
    ) -> SnapshotResult<()> {
        let data_hex = CoreDbJournal::encode_payload(&snapshot_data);
        let timestamp_str = metadata.timestamp.to_rfc3339();

        let cql = format!(
            "INSERT INTO {}.snapshots (persistence_id, sequence_nr, timestamp, snapshot_data) VALUES ('{}', {}, '{}', '{}')",
            self.keyspace,
            metadata.persistence_id,
            metadata.sequence_nr,
            timestamp_str,
            data_hex
        );

        self.db.execute_cql(&cql).await.map_err(|e| {
            SnapshotError::Save(format!("Failed to save snapshot: {}", e))
        })?;

        info!(
            persistence_id = %metadata.persistence_id,
            sequence_nr = metadata.sequence_nr,
            "Snapshot saved to CoreDB"
        );

        Ok(())
    }

    async fn load_snapshot(
        &self,
        persistence_id: &str,
        max_sequence_nr: Option<u64>,
        _max_timestamp: Option<DateTime<Utc>>,
    ) -> SnapshotResult<Option<(SnapshotMetadata, Vec<u8>)>> {
        // Get the latest snapshot (highest sequence_nr)
        let cql = format!(
            "SELECT persistence_id, sequence_nr, timestamp, snapshot_data FROM {}.snapshots WHERE persistence_id = '{}'",
            self.keyspace, persistence_id
        );

        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => {
                // Filter and find the latest valid snapshot
                let mut best: Option<(SnapshotMetadata, Vec<u8>)> = None;

                for row in &rows {
                    let seq_nr = match row.columns.get("sequence_nr") {
                        Some(CassandraValue::BigInt(n)) => *n as u64,
                        _ => continue,
                    };

                    // Apply max_sequence_nr filter
                    if let Some(max_seq) = max_sequence_nr {
                        if seq_nr > max_seq {
                            continue;
                        }
                    }

                    let timestamp_str = match row.columns.get("timestamp") {
                        Some(CassandraValue::Text(s)) => s.clone(),
                        _ => continue,
                    };
                    let data_hex = match row.columns.get("snapshot_data") {
                        Some(CassandraValue::Text(s)) => s.clone(),
                        _ => continue,
                    };

                    let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now());

                    let should_replace = match &best {
                        None => true,
                        Some((existing, _)) => seq_nr > existing.sequence_nr,
                    };

                    if should_replace {
                        let metadata = SnapshotMetadata {
                            persistence_id: persistence_id.to_string(),
                            sequence_nr: seq_nr,
                            timestamp,
                        };
                        let data = CoreDbJournal::decode_payload(&data_hex);
                        best = Some((metadata, data));
                    }
                }

                Ok(best)
            }
            Ok(_) => Ok(None),
            Err(e) => {
                warn!(error = %e, "Failed to load snapshot from CoreDB");
                Ok(None)
            }
        }
    }

    async fn delete_snapshot(&self, metadata: &SnapshotMetadata) -> SnapshotResult<()> {
        let cql = format!(
            "DELETE FROM {}.snapshots WHERE persistence_id = '{}' AND sequence_nr = {}",
            self.keyspace, metadata.persistence_id, metadata.sequence_nr
        );
        self.db.execute_cql(&cql).await.map_err(|e| {
            SnapshotError::Delete(format!("Failed to delete snapshot: {}", e))
        })?;
        Ok(())
    }

    async fn delete_snapshots(
        &self,
        persistence_id: &str,
        max_sequence_nr: Option<u64>,
        _max_timestamp: Option<DateTime<Utc>>,
    ) -> SnapshotResult<()> {
        let cql = format!(
            "SELECT sequence_nr FROM {}.snapshots WHERE persistence_id = '{}'",
            self.keyspace, persistence_id
        );

        if let Ok(QueryResult::Rows(rows)) = self.db.execute_cql(&cql).await {
            for row in &rows {
                if let Some(CassandraValue::BigInt(seq)) = row.columns.get("sequence_nr") {
                    let should_delete = match max_sequence_nr {
                        Some(max) => (*seq as u64) <= max,
                        None => true,
                    };
                    if should_delete {
                        let delete_cql = format!(
                            "DELETE FROM {}.snapshots WHERE persistence_id = '{}' AND sequence_nr = {}",
                            self.keyspace, persistence_id, seq
                        );
                        let _ = self.db.execute_cql(&delete_cql).await;
                    }
                }
            }
        }

        info!(persistence_id = %persistence_id, "Snapshots deleted from CoreDB");
        Ok(())
    }

    async fn list_snapshots(
        &self,
        persistence_id: &str,
    ) -> SnapshotResult<Vec<SnapshotMetadata>> {
        let cql = format!(
            "SELECT persistence_id, sequence_nr, timestamp FROM {}.snapshots WHERE persistence_id = '{}'",
            self.keyspace, persistence_id
        );

        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => {
                let snapshots: Vec<SnapshotMetadata> = rows
                    .iter()
                    .filter_map(|row| {
                        let seq_nr = match row.columns.get("sequence_nr") {
                            Some(CassandraValue::BigInt(n)) => *n as u64,
                            _ => return None,
                        };
                        let timestamp_str = match row.columns.get("timestamp") {
                            Some(CassandraValue::Text(s)) => s.clone(),
                            _ => return None,
                        };
                        let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp_str)
                            .map(|dt| dt.with_timezone(&Utc))
                            .unwrap_or_else(|_| Utc::now());

                        Some(SnapshotMetadata {
                            persistence_id: persistence_id.to_string(),
                            sequence_nr: seq_nr,
                            timestamp,
                        })
                    })
                    .collect();
                Ok(snapshots)
            }
            Ok(_) => Ok(vec![]),
            Err(e) => Err(SnapshotError::Load(format!("Failed to list snapshots: {}", e))),
        }
    }
}
