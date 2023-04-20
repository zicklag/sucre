//! Utilities used internally

use crate::prld::*;

/// A custom [`Arc`]-like type that represents either a [`UniqueArc`] or a shared [`Arc`].
pub enum DualArc<T> {
    /// A [`UniqueArc`], guaranteed to be the only borrow of the wrapped data.
    Unique(UniqueArc<T>),
    /// A shared [`Arc`], which may be borrowed by more than one thread.
    Shared(Arc<T>),
}

impl<T: std::fmt::Debug> std::fmt::Debug for DualArc<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unique(arg0) => f.debug_tuple("Unique").field(&(**arg0)).finish(),
            Self::Shared(arg0) => f.debug_tuple("Shared").field(arg0).finish(),
        }
    }
}

impl<T> DualArc<T> {
    pub fn unique(&mut self) -> &mut T {
        if let Self::Unique(arc) = self {
            arc
        } else {
            panic!("UniqueOrSharedArc is not Unique");
        }
    }

    pub fn new_unique(t: T) -> Self {
        Self::Unique(UniqueArc::new(t))
    }

    pub fn new_shared(t: T) -> Self {
        Self::Shared(Arc::new(t))
    }

    pub fn make_shared(self) -> (Self, Arc<T>) {
        match self {
            DualArc::Unique(unique) => {
                let arc = unique.shareable();

                (Self::Shared(arc.clone()), arc)
            }
            DualArc::Shared(shared) => (self, shared.clone()),
        }
    }
}

/// Impl `Deref` and `Deref` mut for a tuple struct with a single element.
#[doc(hidden)]
macro_rules! newtype {
    ($t:ident, $target:ty) => {
        impl std::ops::Deref for $t {
            type Target = $target;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
        impl std::ops::DerefMut for $t {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }
    };
}

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
