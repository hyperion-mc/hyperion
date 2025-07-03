pub mod __private {
    use std::iter::Peekable;
    /// Gets the next item from a [`std::iter::Peekable`]. Used for better error messages.
    pub fn peekable_next<I: Iterator>(i: &mut Peekable<I>) -> Option<<I as Iterator>::Item> {
        i.next()
    }

    /// Peeks at the next item from a [`std::iter::Peekable`]. Used for better error messages.
    pub fn peekable_peek<I: Iterator>(i: &mut Peekable<I>) -> Option<&<I as Iterator>::Item> {
        i.peek()
    }
}

/// Gets the next lowest element out of several [`std::iter::Peekable`] iterators.
///
/// # Examples
///
/// ```
/// let a = [2, 1, 3];
/// let b = [3, 2, 3];
/// let c = [0, 3, 4];
/// let d = [1, 2, 3];
/// let mut a_iter = a.into_iter().peekable();
/// let mut b_iter = b.into_iter().peekable();
/// let mut c_iter = c.into_iter().peekable();
/// let mut d_iter = d.into_iter().peekable();
/// let value = hyperion_utils::next_lowest! {
///     x in a_iter => x,
///     x in b_iter => x,
///     x in c_iter => x,
///     x in d_iter => x
/// };
///
/// // The next element of the `c_iter` iterator is the lowest out of all the other iterators
/// assert_eq!(value, Some(c[0]));
///
/// assert_eq!(a_iter.peek(), Some(&a[0]));
/// assert_eq!(b_iter.peek(), Some(&b[0]));
/// assert_eq!(c_iter.peek(), Some(&c[1]));
/// assert_eq!(d_iter.peek(), Some(&d[0]));
/// ```
#[macro_export]
macro_rules! next_lowest {
    {} => {
        None
    };
    {
        $bind:pat in $it:expr => $handler:expr$(,)?
    } => {
        match $crate::iterator::__private::peekable_next(&mut $it) {
            Some($bind) => {
                let _result = $handler;
                #[allow(unreachable_code)]
                Some(_result)
            },
            None => None,
        }
    };
    {
        $bind0:pat in $it0:expr => $handler0:expr,
        $bind1:pat in $it1:expr => $handler1:expr$(,)?
    } => {
        match ($crate::iterator::__private::peekable_peek(&mut $it0), $crate::iterator::__private::peekable_peek(&mut $it1)) {
            (Some(a), Some(b)) => {
                if a < b {
                    $crate::next_lowest! {
                        $bind0 in $it0 => $handler0
                    }
                } else {
                    $crate::next_lowest! {
                        $bind1 in $it1 => $handler1
                    }
                }
            },
            (Some(a), None) => {
                $crate::next_lowest! {
                    $bind0 in $it0 => $handler0
                }
            },
            (None, Some(b)) => {
                $crate::next_lowest! {
                    $bind1 in $it1 => $handler1
                }
            },
            (None, None) => None
        }
    };
    {
        $bind0:pat in $it0:expr => $handler0:expr,
        $bind1:pat in $it1:expr => $handler1:expr,
        $($binds:pat in $its:expr => $handlers:expr),*
    } => {
        match ($crate::iterator::__private::peekable_peek(&mut $it0), $crate::iterator::__private::peekable_peek(&mut $it1)) {
            (Some(a), Some(b)) => {
                if a < b {
                    $crate::next_lowest! {
                        $bind0 in $it0 => $handler0,
                        $($binds in $its => $handlers),*
                    }
                } else {
                    $crate::next_lowest! {
                        $bind1 in $it1 => $handler1,
                        $($binds in $its => $handlers),*
                    }
                }
            },
            (Some(a), None) => {
                $crate::next_lowest! {
                    $bind0 in $it0 => $handler0,
                    $($binds in $its => $handlers),*
                }
            },
            (None, Some(b)) => {
                $crate::next_lowest! {
                    $bind1 in $it1 => $handler1,
                    $($binds in $its => $handlers),*
                }
            },
            (None, None) => {
                $crate::next_lowest! {
                    $($binds in $its => $handlers),*
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    #[test]
    #[expect(clippy::diverging_sub_expression)]
    fn test_empty() {
        #[derive(PartialEq, Debug)]
        enum Never {}

        assert_eq!(next_lowest! {}, None::<Never>);

        assert_eq!(
            next_lowest! { () in std::iter::empty::<()>().peekable() => unreachable!() },
            None::<()>
        );
    }

    #[test]
    #[expect(clippy::diverging_sub_expression)]
    fn test_jumps() {
        next_lowest! {
            _ in std::iter::once(1).peekable() => {
                // This should return from the test_jumps function
                return;
            }
        };
        unreachable!()
    }
}
