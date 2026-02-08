#![no_std]

use core::panic::PanicInfo;
#[unsafe(no_mangle)]
extern "C" fn hello() -> i32 {
    3
}

#[panic_handler]
fn panic_handler(_panic_info: &PanicInfo) -> ! {
    loop {}
}
