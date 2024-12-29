use std::mem::{ManuallyDrop, size_of};

use valence_protocol::packets::play;

unsafe impl Lifetime for play::ChatMessageC2s<'_> {
    type WithLifetime<'a> = play::ChatMessageC2s<'a>;
}

unsafe impl Lifetime for play::RequestCommandCompletionsC2s<'_> {
    type WithLifetime<'a> = play::RequestCommandCompletionsC2s<'a>;
}

unsafe impl<T: Lifetime> Lifetime for &T {
    type WithLifetime<'a> = &'a T::WithLifetime<'a>;
}

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
