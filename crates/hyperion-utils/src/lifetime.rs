use std::mem::{ManuallyDrop, size_of};

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
        // the Lifetime trait ensure that no type cast or cast to a longer lifetime is occuring.
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
