//! Utilities for common archive operations.
//!
//! ## Buffer access
//!
//! Helper functions to get the root object of an archive under certain
//! conditions.
//!
//! ## Alignment
//!
//! Alignment helpers ensure that byte buffers are properly aligned when
//! accessing and deserializing data.

#[cfg(feature = "alloc")]
mod aligned_vec;
mod inline_vec;
mod ser_vec;

use core::{
    mem,
    ops::{Deref, DerefMut},
    pin::Pin,
};

use rancor::Strategy;

#[doc(inline)]
#[cfg(feature = "alloc")]
pub use self::aligned_vec::*;
#[doc(inline)]
pub use self::{inline_vec::InlineVec, ser_vec::SerVec};
#[cfg(feature = "alloc")]
use crate::{de::pooling::Unify, ser::AllocSerializer};
use crate::{ser::Writer, Archive, Deserialize, Portable, Serialize};

#[cfg(debug_assertions)]
#[inline]
fn check_alignment<T: Portable>(ptr: *const u8) {
    let expect_align = core::mem::align_of::<T>();
    let actual_align = (ptr as usize) & (expect_align - 1);
    debug_assert_eq!(
        actual_align,
        0,
        concat!(
            "unaligned buffer, expected alignment {} but found alignment {}\n",
            "help: rkyv requires byte buffers to be aligned to access the \
             data inside.\n",
            "      Using an AlignedVec or manually aligning your data with \
             #[align(...)]\n",
            "      may resolve this issue.",
        ),
        expect_align,
        1 << actual_align.trailing_zeros()
    );
}

/// Accesses an archived value from the given byte slice at the given position.
///
/// This function does not check that the data at the given position is valid.
/// Use [`access_pos`](crate::validation::util::access_pos) to validate the data
/// instead.
///
/// # Safety
///
/// A `T` must be located at the given position in the byte slice.
#[inline]
pub unsafe fn access_pos_unchecked<T: Portable>(
    bytes: &[u8],
    pos: usize,
) -> &T {
    #[cfg(debug_assertions)]
    check_alignment::<T>(bytes.as_ptr());

    &*bytes.as_ptr().add(pos).cast()
}

/// Accesses a mutable archived value from the given byte slice at the given
/// position.
///
/// This function does not check that the data at the given position is valid.
/// Use [`access_pos_mut`](crate::validation::util::access_pos_mut) to validate
/// the data instead.
///
/// # Safety
///
/// A `T` must be located at the given position in the byte slice.
#[inline]
pub unsafe fn access_pos_unchecked_mut<T: Portable>(
    bytes: &mut [u8],
    pos: usize,
) -> Pin<&mut T> {
    #[cfg(debug_assertions)]
    check_alignment::<T>(bytes.as_ptr());

    Pin::new_unchecked(&mut *bytes.as_mut_ptr().add(pos).cast())
}

/// Accesses an archived value from the given byte slice by calculating the root
/// position.
///
/// This is a wrapper for [`access_pos_unchecked`] that calculates the position
/// of the root object using the length of the byte slice. If your byte slice is
/// not guaranteed to end immediately after the root object, you may need to
/// store the position of the root object returned from
/// [`serialize_and_resolve`](crate::Serialize::serialize_and_resolve).
///
/// # Safety
///
/// - The byte slice must represent an archived object.
/// - The root of the object must be stored at the end of the slice (this is the
///   default behavior).
#[inline]
pub unsafe fn access_unchecked<T: Portable>(bytes: &[u8]) -> &T {
    access_pos_unchecked::<T>(bytes, bytes.len() - mem::size_of::<T>())
}

/// Accesses a mutable archived value from the given byte slice by calculating
/// the root position.
///
/// This is a wrapper for [`access_pos_unchecked_mut`] that calculates the
/// position of the root object using the length of the byte slice. If your byte
/// slice is not guaranteed to end immediately after the root object, you may
/// need to store the position of the root object returned from
/// [`serialize_and_resolve`](crate::Serialize::serialize_and_resolve).
///
/// # Safety
///
/// - The byte slice must represent an archived object.
/// - The root of the object must be stored at the end of the slice (this is the
///   default behavior).
#[inline]
pub unsafe fn access_unchecked_mut<T: Portable>(
    bytes: &mut [u8],
) -> Pin<&mut T> {
    let pos = bytes.len() - mem::size_of::<T>();
    access_pos_unchecked_mut::<T>(bytes, pos)
}

/// A buffer of bytes aligned to 16 bytes.
///
/// # Examples
///
/// ```
/// # use rkyv::util::AlignedBytes;
/// use core::mem;
///
/// assert_eq!(mem::align_of::<u8>(), 1);
/// assert_eq!(mem::align_of::<AlignedBytes<256>>(), 16);
/// ```
// TODO: re-enable archive derives
#[derive(Archive, Clone, Copy, Debug, Deserialize, Portable, Serialize)]
#[archive(crate)]
#[repr(C, align(16))]
pub struct AlignedBytes<const N: usize>(pub [u8; N]);

impl<const N: usize> Default for AlignedBytes<N> {
    fn default() -> Self {
        Self([0; N])
    }
}

impl<const N: usize> Deref for AlignedBytes<N> {
    type Target = [u8; N];

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const N: usize> DerefMut for AlignedBytes<N> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<const N: usize> AsRef<[u8]> for AlignedBytes<N> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl<const N: usize> AsMut<[u8]> for AlignedBytes<N> {
    #[inline]
    fn as_mut(&mut self) -> &mut [u8] {
        self.0.as_mut()
    }
}

/// Serializes the given value and returns the resulting bytes.
///
/// The const generic parameter `N` specifies the number of bytes to
/// pre-allocate as scratch space. Choosing a good default value for your data
/// can be difficult without any data, so consider using an
/// [`AllocationTracker`](crate::ser::allocator::AllocationTracker) to determine
/// how much scratch space is typically used.
///
/// This function is only available with the `alloc` feature because it uses a
/// general-purpose serializer. In no-alloc and high-performance environments,
/// the serializer should be customized for the specific situation.
///
/// # Examples
/// ```
/// use rkyv::rancor::Error;
///
/// let value = vec![1, 2, 3, 4];
///
/// let bytes =
///     rkyv::to_bytes::<Error>(&value).expect("failed to serialize vec");
/// // SAFETY:
/// // - The byte slice represents an archived object
/// // - The root of the object is stored at the end of the slice
/// let deserialized = unsafe {
///     rkyv::from_bytes_unchecked::<Vec<i32>, Error>(&bytes)
///         .expect("failed to deserialize vec")
/// };
///
/// assert_eq!(deserialized, value);
/// ```
#[cfg(feature = "alloc")]
#[inline]
pub fn to_bytes<E>(
    value: &impl Serialize<Strategy<AllocSerializer, E>>,
) -> Result<AlignedVec, E> {
    Ok(serialize_into(value, Default::default())?.into_writer())
}

/// Serializes the given value into the given serializer and then returns the
/// serializer.
#[inline]
pub fn serialize_into<S, E>(
    value: &impl Serialize<Strategy<S, E>>,
    mut serializer: S,
) -> Result<S, E>
where
    S: Writer<E>,
{
    serialize(value, &mut serializer)?;
    Ok(serializer)
}

/// Serializes the given value into the given serializer.
#[inline]
pub fn serialize<S, E>(
    value: &impl Serialize<Strategy<S, E>>,
    serializer: &mut S,
) -> Result<(), E>
where
    S: Writer<E> + ?Sized,
{
    value.serialize_and_resolve(Strategy::wrap(serializer))?;
    Ok(())
}

/// Deserializes a value from the given bytes.
///
/// This function is only available with the `alloc` feature because it uses a
/// general-purpose deserializer. In no-alloc and high-performance environments,
/// the deserializer should be customized for the specific situation.
///
/// # Safety
///
/// - The byte slice must represent an archived object.
/// - The root of the object must be stored at the end of the slice (this is the
///   default behavior).
///
/// # Examples
/// ```
/// use rkyv::rancor::Error;
///
/// let value = vec![1, 2, 3, 4];
///
/// let bytes =
///     rkyv::to_bytes::<Error>(&value).expect("failed to serialize vec");
/// // SAFETY:
/// // - The byte slice represents an archived object
/// // - The root of the object is stored at the end of the slice
/// let deserialized = unsafe {
///     rkyv::from_bytes_unchecked::<Vec<i32>, Error>(&bytes)
///         .expect("failed to deserialize vec")
/// };
///
/// assert_eq!(deserialized, value);
/// ```
#[cfg(feature = "alloc")]
#[inline]
pub unsafe fn from_bytes_unchecked<T, E>(bytes: &[u8]) -> Result<T, E>
where
    T: Archive,
    T::Archived: Deserialize<T, Strategy<Unify, E>>,
{
    deserialize(
        access_unchecked::<T::Archived>(bytes),
        &mut Unify::default(),
    )
}

/// TODO: document
#[inline]
pub fn deserialize<T, D, E>(
    value: &T::Archived,
    deserializer: &mut D,
) -> Result<T, E>
where
    T: Archive,
    T::Archived: Deserialize<T, Strategy<D, E>>,
{
    value.deserialize(Strategy::wrap(deserializer))
}
