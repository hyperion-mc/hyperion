use std::{
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

/// # Safety
/// The type of [`Lifetime::WithLifetime`] must be the same type as `Self` aside from lifetimes. In
/// addition, [`Lifetime::WithLifetime`] may not use `'static` in a lifetime parameter if the original
/// `Self` type did not use `'static` in the same lifetime parameters.
pub unsafe trait Lifetime {
    type WithLifetime<'a>: Lifetime + 'a;

    /// # Safety
    /// This may change references to have the lifetime of `'a`, which may impose additional safety
    /// requirements.
    #[must_use]
    unsafe fn change_lifetime<'a>(self) -> Self::WithLifetime<'a>
    where
        Self: Sized,
    {
        // SAFETY: This lifetime cast is checked by the caller, and the safety requirements on implementors of
        // the [`Lifetime`] trait ensure that no type cast or cast to a longer lifetime is occuring.
        unsafe { transmute_unchecked(self) }
    }

    fn shorten_lifetime<'a>(self) -> Self::WithLifetime<'a>
    where
        Self: 'a + Sized,
    {
        // SAFETY: Shortening a lifetime is allowed
        unsafe { self.change_lifetime() }
    }

    fn shorten_lifetime_ref<'a>(&self) -> &Self::WithLifetime<'a>
    where
        Self: 'a + Sized,
    {
        // SAFETY: Shortening a lifetime is allowed
        unsafe { &*std::ptr::from_ref(self).cast::<Self::WithLifetime<'a>>() }
    }
}

unsafe impl<T> Lifetime for &T
where
    T: ?Sized + 'static,
{
    type WithLifetime<'a> = &'a T;
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
    pub fn new<'a>(
        value: T,
        handle: &dyn LifetimeHandle<'a>,
    ) -> RuntimeLifetime<T::WithLifetime<'static>>
    where
        T: 'a,
    {
        // SAFETY: [`RuntimeLifetime::get`] ensures that the `'static` referencs are not exposed
        // publicly and that users can only access `T` with an appropriate lifetime.
        let value = unsafe { value.change_lifetime::<'static>() };

        let references = unsafe { handle.__private_references(sealed::Sealed) };
        references.fetch_add(1, Ordering::SeqCst);

        RuntimeLifetime {
            value,
            references: std::ptr::from_ref(references),
        }
    }

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

unsafe impl<T> Send for RuntimeLifetime<T> where T: Send {}
unsafe impl<T> Sync for RuntimeLifetime<T> where T: Sync {}

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
    pub struct Sealed;
}

pub trait LifetimeHandle<'a> {
    /// # Safety
    /// The returned references value must only be used in increment-decrement pairs. In other words, it can only be
    /// decremented if it were previously incremented.
    #[must_use]
    unsafe fn __private_references(&self, _: sealed::Sealed) -> &AtomicUsize;
}

struct LifetimeHandleObject<'a> {
    references: &'a AtomicUsize,
}

impl<'a> LifetimeHandle<'a> for LifetimeHandleObject<'a> {
    unsafe fn __private_references(&self, _: sealed::Sealed) -> &AtomicUsize {
        self.references
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
    /// Data which outlives the `'handle` lifetime and might have a [`RuntimeLifetime`] constructed with the resulting
    /// [`LifetimeHandle`] must only be dropped after [`LifetimeTracker::assert_no_references`] is called on this
    /// tracker. The only purpose of the `'handle` lifetime is to allow users to control which values can be wrapped
    /// in a [`RuntimeLifetime`] since wrapped values must outlive `'handle`. The length of the `'handle` lifetime
    /// itself does not matter, and `'handle` may expire before [`LifetimeTracker::assert_no_references`] is called.
    #[must_use]
    pub unsafe fn handle<'handle>(&'handle self) -> impl LifetimeHandle<'handle> {
        // Storing the lifetime parameter in a trait ([`LifetimeHandle`]) instead of a struct is necessary to prohibit
        // casts to a shorter lifetime. If the [`LifetimeHandle`]'s lifetime could be shortened, the user could safely
        // wrap values of any lifetime in [`RuntimeLifetime`], which would defeat the purpose of the `'handle` lifetime.
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
