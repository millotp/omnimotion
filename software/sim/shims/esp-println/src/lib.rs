//! The firmware's console goes to the simulator log.

#[doc(hidden)]
pub fn _print(args: core::fmt::Arguments<'_>) {
    omnimotion_sim::hal::console(&std::fmt::format(args));
}

#[macro_export]
macro_rules! println {
    () => { $crate::_print(format_args!("")) };
    ($($arg:tt)*) => { $crate::_print(format_args!($($arg)*)) };
}
