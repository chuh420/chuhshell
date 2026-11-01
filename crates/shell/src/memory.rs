use std::ffi::c_int;

const M_MMAP_THRESHOLD: c_int = -3;

unsafe extern "C" {
    fn mallopt(param: c_int, value: c_int) -> c_int;
    fn malloc_trim(pad: usize) -> c_int;
}

pub fn configure() {
    if unsafe { mallopt(M_MMAP_THRESHOLD, 128 * 1024) } == 0 {
        log::warn!("mallopt failed");
    }
}

pub fn trim() {
    unsafe {
        malloc_trim(0);
    }
}
