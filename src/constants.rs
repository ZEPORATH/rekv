pub const DEFAULT_CONFIG_PATH: &str = "/var/lib/rekv/state.json";
pub const DEFAULT_UDS_PATH: &str = "/tmp/rekv.sock";
pub const DEFAULT_GRPC_PORT: u16 = 50051;
pub const DEFAULT_LOCAL_HOST: &str = "127.0.0.1";
pub const DEFAULT_BIND_HOST: &str = "0.0.0.0";
pub const DEFAULT_GRPC_ADDRESS: &str = "127.0.0.1:50051";
pub const DEFAULT_GRPC_URL: &str = "http://127.0.0.1:50051";

pub const ROOT_PATH: &str = "/";
pub const PATH_SEPARATOR_CHAR: char = '/';
pub const PATH_SEPARATOR_STR: &str = "/";
pub const PRIMARY_KEY_ATTRIBUTE: &str = "id";
pub const PRIMARY_KEY_SHORTHAND_PREFIX: char = '#';
pub const WILDCARD_SINGLE: &str = "*";
pub const WILDCARD_RECURSIVE: &str = "**";

pub const OP_GTE: &str = ">=";
pub const OP_LTE: &str = "<=";
pub const OP_NEQ: &str = "!=";
pub const OP_EQ_DOUBLE: &str = "==";
pub const OP_EQ: &str = "=";
pub const OP_GT: &str = ">";
pub const OP_LT: &str = "<";

pub const JSON_NULL: &str = "null";
pub const JSON_TRUE: &str = "true";
pub const JSON_FALSE: &str = "false";
pub const TEMP_FILE_SUFFIX: &str = ".tmp";

pub const DEFAULT_BROADCAST_CHANNEL_CAPACITY: usize = 256;

