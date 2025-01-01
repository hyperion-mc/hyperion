use std::{
    marker::PhantomData,
    mem::{ManuallyDrop, size_of},
    sync::atomic::{AtomicUsize, Ordering},
};

/// # Safety
/// Same safety requirements as [`std::mem::transmute`]. In addition, both types must have the same
/// size, but this is not checked at compile time.
unsafe fn transmute_unchecked<Src, Dst>(src: Src) -> Dst {
    debug_assert_eq!(size_of::<Src>(), size_of::<Dst>());
    let src = ManuallyDrop::new(src);
    // SAFETY: ensured by caller
    unsafe { std::ptr::read(std::ptr::from_ref(&src).cast()) }
}

// TODO: create derive macro to implement Lifetime
/// # Safety
/// The type of [`Lifetime::WithLifetime`] must be the same type as `Self` aside from lifetimes. In
/// addition, [`Lifetime::WithLifetime`] may not use `'static` in a lifetime parameter if the original
/// `Self` type did not use `'static` in the same lifetime parameters.
pub unsafe trait Lifetime {
    type WithLifetime<'a>: Lifetime + 'a;

    // TODO: this function is unused, but deleting it causes a "missing required bound" error?
    #[must_use]
    fn _unused<'a>() -> Self::WithLifetime<'a> {
        unimplemented!()
    }

    fn shorten_lifetime<'a>(self) -> Self::WithLifetime<'a>
    where
        Self: 'a + Sized,
    {
        // SAFETY: Shortening a lifetime is allowed, and the safety requirements on implementors of
        // the [`Lifetime`] trait ensure that no type cast or cast to a longer lifetime is occuring.
        unsafe { transmute_unchecked(self) }
    }
}

unsafe impl<T: Lifetime> Lifetime for &T {
    type WithLifetime<'a> = &'a T::WithLifetime<'a>;
}

hyperion_packet_macros::for_each_static_play_c2s_packet! {
    unsafe impl Lifetime for PACKET {
        type WithLifetime<'a> = PACKET;
    }
}

hyperion_packet_macros::for_each_lifetime_play_c2s_packet! {
    unsafe impl Lifetime for PACKET<'_> {
        type WithLifetime<'a> = PACKET<'a>;
    }
}

pub struct RuntimeLifetime<T> {
    value: T,
    references: *const AtomicUsize,
}

impl<T: Lifetime> RuntimeLifetime<T> {
    #[must_use]
    pub const fn get<'a>(&'a self) -> &'a T::WithLifetime<'a> {
        // SAFETY: The program will abort if `self` is not dropped before
        // [`LifetimeTracker::assert_no_references`] is called. `'a` will expire before `self` is
        // dropped. Therefore, it is safe to change these references to `'a`, because if `'a`
        // were to live after [`LifetimeTracker::assert_no_references`] is called, the program
        // would abort before user code could use the invalid reference.
        unsafe { &*(&raw const self.value).cast::<T::WithLifetime<'a>>() }
    }
}

impl<T> Drop for RuntimeLifetime<T> {
    fn drop(&mut self) {
        // SAFETY: `self.references` is safe to dereference because the underlying [`LifetimeTracker`] would
        // have already aborted if it were dropped before this
        unsafe {
            (*self.references).fetch_sub(1, Ordering::Relaxed);
        }
    }
}

mod sealed {
    pub trait Sealed {}
}

pub trait LifetimeHandle<'a>: sealed::Sealed + Copy + Clone {
    #[must_use]
    fn runtime_lifetime<T>(&self, value: T) -> RuntimeLifetime<T>
    where
        T: 'a;
}

#[derive(Copy, Clone)]
struct LifetimeHandleObject<'a> {
    references: &'a AtomicUsize,
}
impl sealed::Sealed for LifetimeHandleObject<'_> {}
impl<'a> LifetimeHandle<'a> for LifetimeHandleObject<'a> {
    fn runtime_lifetime<T>(&self, value: T) -> RuntimeLifetime<T>
    where
        T: 'a,
    {
        self.references.fetch_add(1, Ordering::SeqCst);

        RuntimeLifetime {
            value,
            references: std::ptr::from_ref(self.references),
        }
    }
}

pub struct LifetimeTracker {
    references: Box<AtomicUsize>,
}

impl LifetimeTracker {
    pub fn assert_no_references(&mut self) {
        // TODO: determine better ordering to use
        let references = self.references.load(Ordering::SeqCst);
        if references != 0 {
            tracing::error!("{references} values were held too long - aborting");
            // abort is needed to avoid a panic handler allowing those values to continue being
            // used
            std::process::abort();
        }
    }

    /// # Safety
    /// Data which might be passed to the resulting [`LifetimeHandle::runtime_lifetime`] and outlives the `'handle`
    /// lifetime must only be dropped after [`LifetimeTracker::assert_no_references`] is called on this tracker. The
    /// only purpose of the `'handle` lifetime is to allow users to control which values are allowed to be passed to
    /// [`LifetimeHandle::runtime_lifetime`] since values passed there must outlive `'handle`. The length of the
    /// `'handle` lifetime itself does not matter, and `'handle` may expire before
    /// [`LifetimeTracker::assert_no_references`] is called.
    ///
    /// It is very important to **specify the `'handle` lifetime parameter**. Allowing `'handle` to be
    /// inferred is likely to violate the above safety requirements.
    #[must_use]
    unsafe fn handle<'handle>(&'handle self) -> impl LifetimeHandle<'handle> {
        // Storing the lifetime generic in a trait ([`LifetimeHandle`]) instead of a struct is necessary to prohibit
        // casts to a shorter lifetime. If the [`LifetimeHandle`]'s lifetime could be shortened, the user could safely
        // pass values of any lifetime to [`LifetimeHandle::runtime_lifetime`], which would defeat the purpose of the
        // `'handle` lifetime.
        LifetimeHandleObject::<'handle> {
            references: &self.references,
        }
    }
}

impl Default for LifetimeTracker {
    fn default() -> Self {
        Self {
            references: Box::new(AtomicUsize::new(0)),
        }
    }

}

impl Drop for LifetimeTracker {
    fn drop(&mut self) {
        // Even if data associated with this tracker will live for `'static`, the [`Box`] storing
        // the references will be dropped, so this ensures that there are no
        // [`RuntimeLifetime`]s which might still have a pointer to the references.
        self.assert_no_references();
    }
}
