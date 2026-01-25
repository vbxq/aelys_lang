// platform-specific .init_array / __mod_init_func / .CRT$XCU hooks
// runs at load time to compute exports_hash

#[macro_export]
macro_rules! aelys_init_exports_hash {
    ($descriptor:ident) => {
        #[cfg(any(
            target_os = "linux",
            target_os = "android",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "dragonfly"
        ))]
        #[used]
        #[unsafe(link_section = ".init_array")]
        static AELYS_INIT_EXPORTS_HASH: extern "C" fn() = {
            extern "C" fn init() {
                unsafe {
                    $crate::init_descriptor_exports_hash(&mut $descriptor as *mut _);
                }
            }
            init
        };

        #[cfg(target_os = "macos")]
        #[used]
        #[unsafe(link_section = "__DATA,__mod_init_func")]
        static AELYS_INIT_EXPORTS_HASH: extern "C" fn() = {
            extern "C" fn init() {
                unsafe {
                    $crate::init_descriptor_exports_hash(&mut $descriptor as *mut _);
                }
            }
            init
        };

        #[cfg(target_os = "windows")]
        #[used]
        #[unsafe(link_section = ".CRT$XCU")]
        static AELYS_INIT_EXPORTS_HASH: extern "C" fn() = {
            extern "C" fn init() {
                unsafe {
                    $crate::init_descriptor_exports_hash(&mut $descriptor as *mut _);
                }
            }
            init
        };
    };
}
