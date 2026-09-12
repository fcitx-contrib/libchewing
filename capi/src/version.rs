use std::ffi::{c_char, c_int};

pub const CHEWING_VERSION_MAJOR: c_int = 0;
pub const CHEWING_VERSION_MINOR: c_int = 14;
pub const CHEWING_VERSION_PATCH: c_int = 0;

macro_rules! env_c_ptr {
    ($name:expr) => {{
        const BUF_LEN: usize = env!($name).len() + 1;
        const BUF: [u8; BUF_LEN] = {
            let ver_bytes = env!($name).as_bytes();
            let mut buf = [0u8; BUF_LEN];
            let mut i = 0;
            loop {
                if i >= ver_bytes.len() {
                    break;
                }
                buf[i] = ver_bytes[i];
                i += 1;
            }
            buf
        };
        BUF.as_ptr().cast()
    }};
}

#[unsafe(no_mangle)]
pub extern "C" fn chewing_version() -> *const c_char {
    env_c_ptr!("CARGO_PKG_VERSION")
}

#[unsafe(no_mangle)]
pub extern "C" fn chewing_version_major() -> c_int {
    CHEWING_VERSION_MAJOR
}

#[unsafe(no_mangle)]
pub extern "C" fn chewing_version_minor() -> c_int {
    CHEWING_VERSION_MINOR
}

#[unsafe(no_mangle)]
pub extern "C" fn chewing_version_patch() -> c_int {
    CHEWING_VERSION_PATCH
}

#[unsafe(no_mangle)]
pub extern "C" fn chewing_version_extra() -> *const c_char {
    env_c_ptr!("CARGO_PKG_VERSION_PRE")
}
