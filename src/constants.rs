pub const DEFAULT_CONFIG_PATH: &str = "/var/lib/rekv/state.json";
pub const DEFAULT_UDS_PATH: &str = "/tmp/rekv.sock";
pub const DEFAULT_GRPC_PORT: u16 = 50051;
pub const DEFAULT_HTTP_PORT: u16 = 8080;
pub const DEFAULT_LOCAL_HOST: &str = "127.0.0.1";
pub const DEFAULT_BIND_HOST: &str = "0.0.0.0";
pub const DEFAULT_GRPC_ADDRESS: &str = "127.0.0.1:50051";
pub const DEFAULT_GRPC_URL: &str = "http://127.0.0.1:50051";

pub const ROOT_PATH: &str = "/";
pub const PATH_SEPARATOR_CHAR: char = '/';
pub const PRIMARY_KEY_ATTRIBUTE: &str = "id";
pub const WILDCARD_SINGLE: &str = "*";

pub const JSON_NULL: &str = "null";
pub const TEMP_FILE_SUFFIX: &str = ".tmp";

pub const DEFAULT_BROADCAST_CHANNEL_CAPACITY: usize = 256;
