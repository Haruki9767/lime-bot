pub mod format;
pub mod safe_url;
pub mod validation;

pub use format::{
    MAX_DISCORD_MESSAGE_CHARS, escape_discord_text, format_code_block, format_uptime,
    truncate_discord_text,
};
pub use safe_url::{
    DNS_RESOLUTION_TIMEOUT, SafeHttpUrl, SafeRequestError, SafeUrlError, is_public_ip,
    resolve_safe_http_url,
};
pub use validation::{Host, ValidationError, validate_domain, validate_host};
