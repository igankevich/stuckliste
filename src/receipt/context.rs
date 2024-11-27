use std::collections::HashMap;
use std::ffi::CString;

pub struct Context {
    /// Metadata block index to file size mapping.
    pub file_size_64: HashMap<u32, u64>,

    /// Metadata block index to path mapping.
    pub hard_links: HashMap<u32, CString>,
}

impl Context {
    pub fn new() -> Self {
        Self {
            file_size_64: Default::default(),
            hard_links: Default::default(),
        }
    }
}
