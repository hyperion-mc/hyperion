use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::mem::{size_of, transmute, ManuallyDrop};
use tracing::warn;

unsafe fn transmute_unchecked<Src, Dst>(src: Src) -> Dst {
    debug_assert_eq!(size_of::<Src>(), size_of::<Dst>());
    let src = ManuallyDrop::new(src);
    // SAFETY: ensured by caller
    unsafe {
        std::ptr::read(&src as *const _ as *const Dst)
    }
}

/// # Safety
/// The types of [`Lifetime::WithLifetime`] and [`Lifetime::Static`] must be the same type as
/// `Self` aside from lifetimes
pub unsafe trait Lifetime {
    type WithLifetime<'a>;
    type Static: 'static;
}

pub struct RuntimeCheckedLifetime<T> {
    value: T,
    count: *const AtomicUsize
}

impl<T: Lifetime> RuntimeCheckedLifetime<T> {
    pub fn new<'a>(value: T, tracker: &LifetimeTracker<'a>) -> RuntimeCheckedLifetime<T::Static> where T: 'a {
        // SAFETY: Transmuting to a 'static lifetime is legal as long as the reference is never
        // used after the 'a lifetime. The counter in [`LifetimeTracker`] ensures that the returned
        // [`RuntimeCheckedLifetime`] lives shorter than 'a, and [`RuntimeCheckedLifetime::get`]
        // ensures that the user never receives any references longer than the lifetime of the
        // returned [`RuntimeCheckedLifetime`]

        let value = unsafe { transmute_unchecked::<T, T::Static>(value) };

        tracker.count.fetch_add(1, Ordering::Relaxed);
        RuntimeCheckedLifetime {
            value,
            count: &tracker.count
        }
    }

    pub fn get<'a>(&'a self) -> &'a T::WithLifetime<'a> {
        // SAFETY: The associated [`LifetimeTracker`] will abort if 'a outlives it
        unsafe { transmute::<&'a T, &'a T::WithLifetime<'a>>(&self.value) }
    }
}

impl<T> Drop for RuntimeCheckedLifetime<T> {
    fn drop(&mut self) {
        // SAFETY: self.count is safe to dereference because the underlying LifetimeTracker would
        // have already aborted if it were dropped before the count was decremented
        unsafe {
            (*self.count).fetch_sub(1, Ordering::Relaxed);
        }
    }
}

// TODO: use pin
// TODO: check if atomic needed
pub struct LifetimeTracker<'a> {
    count: AtomicUsize,
    _phantom: PhantomData<&'a ()>
}

impl<'a> LifetimeTracker<'a> {
    /// # Safety
    /// The returned [`LifetimeTracker`] must be dropped. Using [`std::mem::forget`] and variants
    /// is not allowed.
    pub unsafe fn new() -> LifetimeTracker<'a> {
        Self {
            count: AtomicUsize::new(0),
            _phantom: PhantomData
        }
    }
}

impl<'a> Drop for LifetimeTracker<'a> {
    fn drop(&mut self) {
        let count = self.count.get_mut();
        if *count != 0 {
            tracing::error!("{count} values were held past their lifetime - aborting");
            // abort is needed to avoid a panic handler allowing those values to continue being
            // used
            std::process::abort();
        }
    }
}
