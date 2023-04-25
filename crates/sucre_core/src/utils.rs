//! Utilities used internally

// This wasn't used, but might be useful later.
// /// Impl `Deref` and `Deref` mut for a tuple struct with a single element.
// #[doc(hidden)]
// macro_rules! newtype {
//     ($t:ident, $target:ty) => {
//         impl std::ops::Deref for $t {
//             type Target = $target;

//             fn deref(&self) -> &Self::Target {
//                 &self.0
//             }
//         }
//         impl std::ops::DerefMut for $t {
//             fn deref_mut(&mut self) -> &mut Self::Target {
//                 &mut self.0
//             }
//         }
//     };
// }

pub(crate) struct DebugTimer {
    start: std::time::Instant,
    message: &'static str,
}
impl DebugTimer {
    #[allow(unused)]
    pub fn new(message: &'static str) -> Self {
        DebugTimer {
            start: std::time::Instant::now(),
            message,
        }
    }
}
impl Drop for DebugTimer {
    fn drop(&mut self) {
        eprintln!(
            "{}: {:?}",
            self.message,
            std::time::Instant::now() - self.start
        )
    }
}

#[allow(unused)]
macro_rules! debug_timer {
    ($message:expr) => {
        let _ = crate::utils::DebugTimer::new($message);
    };
}
