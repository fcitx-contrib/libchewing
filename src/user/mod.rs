pub(crate) mod dict;
pub(crate) mod freq;
pub(crate) mod history_dict;
pub(crate) mod history_freq;
pub(crate) mod indexed_dict;

pub use self::dict::{UserDict, UserDictError};
pub use self::freq::{UserFreq, UserFreqError};
pub use self::history_dict::{HistoryDict, HistoryDictError};
pub use self::history_freq::{HistoryFreq, HistoryFreqError};

use self::indexed_dict::IndexedDict;
