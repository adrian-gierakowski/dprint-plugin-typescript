pub mod string_utils;

pub fn is_debug() -> bool {
    std::env::var("DPRINT_DEBUG").is_ok()
}
