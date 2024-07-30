
extern "sysv64" {
    pub fn _cs_push(scheduler: extern "sysv64" fn(rsp: *const u8)->!) -> ();
    pub fn _cs_pop(rsp: *const u8) -> !;
}
