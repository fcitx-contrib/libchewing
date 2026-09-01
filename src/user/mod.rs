pub(crate) mod dict;
pub(crate) mod history_dict;
pub(crate) mod indexed_dict;
pub(crate) mod migrate;

pub use self::dict::{UserDict, UserDictError};
pub use self::history_dict::{HistoryDict, HistoryDictError};
pub use self::migrate::MigrateV4Error;
pub use self::migrate::migrate_v3_to_v4;
pub use self::migrate::should_migrate_v3;
