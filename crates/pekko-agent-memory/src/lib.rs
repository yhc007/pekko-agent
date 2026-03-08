pub mod conversation;
pub mod vector_store;
pub mod episodic;

// CoreDB-backed implementations (replaces Redis/PostgreSQL)
pub mod coredb_conversation;
pub mod coredb_episodic;
pub mod coredb_journal;

pub use conversation::*;
pub use vector_store::*;
pub use episodic::*;

// CoreDB exports
pub use coredb_conversation::CoreDbConversationStore;
pub use coredb_episodic::CoreDbEpisodicStore;
pub use coredb_journal::{CoreDbJournal, CoreDbSnapshotStore};
