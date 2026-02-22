//! Fixed-capacity vector implementation backed by a stack-allocated array.
//!
//! This module provides [`ArrayVec`], a vector-like data structure with a compile-time
//! fixed capacity. Unlike `Vec` from the standard library, `ArrayVec` stores its elements
//! inline in a fixed-size array on the stack, avoiding heap allocation entirely.
//!
//! # Features
//!
//! - Zero heap allocations
//! - Compile-time fixed capacity
//! - API similar to standard `Vec`
//! - Works in `no_std` environments
//! - Safe handling of uninitialized memory
//!
//! # Capacity Management
//!
//! The capacity is specified as a const generic parameter and cannot be changed at runtime.
//! Attempting to push beyond capacity will either return an error ([`ArrayVec::try_push`])
//! or panic ([`ArrayVec::push`]).
//!
//! # Performance
//!
//! - Push/pop operations: O(1)
//! - Indexing: O(1)
//! - Better cache locality than heap-allocated vectors
//! - No allocator overhead
//!
//! # Examples
//!
//! ```
//! use planck_noalloc::vec::ArrayVec;
//!
//! let mut vec = ArrayVec::<i32, 10>::new();
//!
//! // Push elements
//! vec.push(1);
//! vec.push(2);
//! vec.push(3);
//!
//! // Access elements
//! assert_eq!(vec[0], 1);
//! assert_eq!(vec.len(), 3);
//!
//! // Iterate
//! for value in vec.iter() {
//!     println!("{}", value);
//! }
//!
//! // Get as slice
//! let slice = vec.as_slice();
//! assert_eq!(slice, &[1, 2, 3]);
//! ```

use core::mem::MaybeUninit;

/// A fixed-size array, which has vector-like operations.
///
/// `ArrayVec` provides a vector-like interface with a compile-time fixed capacity `N`.
/// Elements are stored in a stack-allocated array, making it suitable for `no_std`
/// environments and situations where heap allocation is not available or desirable.
///
/// # Type Parameters
///
/// - `T`: The type of elements stored in the vector
/// - `N`: The maximum number of elements (capacity)
///
/// # Examples
///
/// ```
/// use planck_noalloc::vec::ArrayVec;
///
/// // Create a vector with capacity 4
/// let mut vec = ArrayVec::<String, 4>::new();
/// vec.push(String::from("hello"));
/// vec.push(String::from("world"));
///
/// assert_eq!(vec.len(), 2);
/// ```
#[derive(Debug)]
pub struct ArrayVec<T, const N: usize> {
    data: [MaybeUninit<T>; N],
    len: usize,
}

/// Errors that can occur when operating on an [`ArrayVec`].
#[derive(Debug, Clone)]
pub enum ArrayVecError {
    /// The operation would exceed the fixed capacity of the `ArrayVec`.
    CapacityOverflow,
}

impl core::fmt::Display for ArrayVecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::CapacityOverflow => f.write_str("capacity overflow"),
        }
    }
}

impl core::error::Error for ArrayVecError {}

impl<T, const N: usize> Default for ArrayVec<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> ArrayVec<T, N> {
    /// Creates a new `ArrayVec` with no elements.
    ///
    /// # Examples
    ///
    /// ```
    /// use planck_noalloc::vec::ArrayVec;
    ///
    /// let mut vec = ArrayVec::<u8, 4>::new();
    /// assert_eq!(vec.len(), 0);
    /// assert!(vec.is_empty());
    /// ```
    #[must_use]
    pub const fn new() -> Self {
        Self {
            data: [const { MaybeUninit::uninit() }; N],
            len: 0,
        }
    }

    /// Tries to push a value into the `ArrayVec`.
    ///
    /// # Errors
    ///
    /// Returns an error if the `ArrayVec` is full.
    ///
    /// # Examples
    ///
    /// ```
    /// use planck_noalloc::vec::ArrayVec;
    ///
    /// let mut vec = ArrayVec::<u8, 1>::new();
    /// assert!(vec.try_push(1).is_ok());
    /// assert_eq!(vec.len(), 1);
    /// assert!(vec.try_push(2).is_err());
    /// ```
    pub fn try_push(&mut self, value: T) -> Result<(), ArrayVecError> {
        if self.len == N {
            return Err(ArrayVecError::CapacityOverflow);
        }
        self.data[self.len].write(value);
        self.len += 1;
        Ok(())
    }

    /// Pushes a value into the `ArrayVec`.
    ///
    /// # Panics
    ///
    /// Panics if the `ArrayVec` is full.
    pub fn push(&mut self, value: T) {
        self.try_push(value).expect("ArrayVec: ran out of capacity");
    }

    /// Removes and returns the last element, or `None` if empty.
    #[must_use]
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        // SAFETY: The element at `self.len` was initialized by a previous push.
        Some(unsafe { self.data[self.len].assume_init_read() })
    }

    /// Removes all elements.
    pub fn clear(&mut self) {
        while self.pop().is_some() {}
    }

    /// Removes the element at `index` by swapping it with the last element.
    ///
    /// # Panics
    ///
    /// Panics if `index >= len`.
    pub fn swap_remove(&mut self, index: usize) -> T {
        assert!(index < self.len, "index out of bounds");
        self.len -= 1;
        // Swap the target with the last element, then read the target.
        self.data.swap(index, self.len);
        // SAFETY: The element at `self.len` (formerly at `index`) was initialized.
        unsafe { self.data[self.len].assume_init_read() }
    }

    /// Returns the number of elements in the `ArrayVec`.
    ///
    /// # Examples
    ///
    /// ```
    /// use planck_noalloc::vec::ArrayVec;
    ///
    /// let mut vec = ArrayVec::<u8, 4>::new();
    /// assert_eq!(vec.len(), 0);
    /// vec.push(1);
    /// assert_eq!(vec.len(), 1);
    /// vec.push(2);
    /// assert_eq!(vec.len(), 2);
    /// ```
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if the `ArrayVec` is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use planck_noalloc::vec::ArrayVec;
    ///
    /// let mut vec = ArrayVec::<u8, 4>::new();
    /// assert!(vec.is_empty());
    /// vec.push(1);
    /// assert!(!vec.is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns a slice of all the elements in the `ArrayVec`.
    ///
    /// # Examples
    ///
    /// ```
    /// use planck_noalloc::vec::ArrayVec;
    ///
    /// let mut vec = ArrayVec::<u8, 4>::new();
    /// vec.push(1);
    /// vec.push(2);
    /// vec.push(3);
    /// vec.push(4);
    ///
    /// assert_eq!(vec.as_slice(), &[1, 2, 3, 4]);
    /// ```
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        // SAFETY: Elements 0..self.len are initialized by the push invariant,
        // and the pointer from `self.data` is valid for `self.len` elements.
        unsafe { core::slice::from_raw_parts(self.data.as_ptr().cast::<T>(), self.len) }
    }

    /// Returns a mutable slice of all the elements in the `ArrayVec`.
    ///
    /// # Examples
    ///
    /// ```
    /// use planck_noalloc::vec::ArrayVec;
    ///
    /// let mut vec = ArrayVec::<u8, 4>::new();
    /// vec.push(1);
    /// vec.push(2);
    /// vec.push(3);
    /// vec.push(4);
    ///
    /// assert_eq!(vec.as_mut_slice(), &mut [1, 2, 3, 4]);
    /// ```
    #[must_use]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        // SAFETY: Elements 0..self.len are initialized by the push invariant,
        // and the mutable pointer from `self.data` is valid for `self.len` elements.
        unsafe { core::slice::from_raw_parts_mut(self.data.as_mut_ptr().cast::<T>(), self.len) }
    }

    /// Returns an iterator over the elements of the `ArrayVec`.
    ///
    /// # Examples
    ///
    /// ```
    /// use planck_noalloc::vec::ArrayVec;
    ///
    /// let mut vec = ArrayVec::<u8, 4>::new();
    /// vec.push(1);
    /// vec.push(2);
    /// vec.push(3);
    /// vec.push(4);
    ///
    /// for i in vec.iter() {
    ///     assert!(*i == 1 || *i == 2 || *i == 3 || *i == 4);
    ///     // do something
    /// }
    /// ```
    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.as_slice().iter()
    }

    /// Returns a mutable iterator over the elements of the `ArrayVec`.
    ///
    /// # Examples
    ///
    /// ```
    /// use planck_noalloc::vec::ArrayVec;
    ///
    /// let mut vec = ArrayVec::<u8, 4>::new();
    /// vec.push(1);
    /// vec.push(2);
    /// vec.push(3);
    /// vec.push(4);
    ///
    /// for i in vec.iter_mut() {
    ///     *i += 1;
    ///     // do something
    /// }
    /// ```
    pub fn iter_mut(&mut self) -> core::slice::IterMut<'_, T> {
        self.as_mut_slice().iter_mut()
    }

    /// Reverses the order of the elements in the `ArrayVec`.
    ///
    /// # Examples
    ///
    /// ```
    /// use planck_noalloc::vec::ArrayVec;
    ///
    /// let mut vec = ArrayVec::<u8, 4>::new();
    /// vec.push(1);
    /// vec.push(2);
    /// vec.push(3);
    /// vec.push(4);
    ///
    /// vec.reverse();
    ///
    /// assert_eq!(vec.as_slice(), &[4, 3, 2, 1]);
    /// ```
    pub fn reverse(&mut self) {
        let len = self.len;
        for i in 0..len / 2 {
            let j = len - i - 1;
            self.data.swap(i, j);
        }
    }

    /// Inserts `value` at `index`, shifting all elements after it to the right.
    ///
    /// # Panics
    ///
    /// Panics if `index > len` or if the `ArrayVec` is full.
    pub fn insert(&mut self, index: usize, value: T) {
        assert!(index <= self.len, "index out of bounds");
        assert!(self.len < N, "ArrayVec: ran out of capacity");
        // SAFETY: We shift elements [index..len] one position right.
        // All elements in that range are initialized. After the shift,
        // index `self.len` is moved from `self.len - 1` (or is the gap
        // at `index`). We then write `value` into the gap at `index`.
        unsafe {
            let ptr = self.data.as_mut_ptr().add(index);
            core::ptr::copy(ptr, ptr.add(1), self.len - index);
            ptr.cast::<T>().write(value);
        }
        self.len += 1;
    }

    /// Removes and returns the element at `index`, shifting all elements
    /// after it to the left. Preserves ordering.
    ///
    /// # Panics
    ///
    /// Panics if `index >= len`.
    pub fn remove(&mut self, index: usize) -> T {
        assert!(index < self.len, "index out of bounds");
        // SAFETY: The element at `index` is initialized. We read it out,
        // then shift elements [index+1..len] one position left.
        unsafe {
            let ptr = self.data.as_mut_ptr().add(index);
            let value = ptr.cast::<T>().read();
            core::ptr::copy(ptr.add(1), ptr, self.len - index - 1);
            self.len -= 1;
            value
        }
    }

    /// Returns the last element, or `None` if empty.
    #[must_use]
    pub fn last(&self) -> Option<&T> {
        if self.len == 0 {
            None
        } else {
            Some(&self[self.len - 1])
        }
    }

    /// Returns true if the `ArrayVec` is at capacity.
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.len == N
    }

    /// Returns a pointer to the underlying data.
    #[must_use]
    pub fn as_ptr(&self) -> *const T {
        self.data.as_ptr().cast::<T>()
    }

    /// Returns a mutable pointer to the underlying data.
    #[must_use]
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.data.as_mut_ptr().cast::<T>()
    }
}

impl<T, const U: usize> ArrayVec<T, U>
where
    T: Copy + PartialEq,
{
    /// Returns true if the `ArrayVec` contains the given value.
    ///
    /// # Examples
    /// ```
    /// use planck_noalloc::vec::ArrayVec;
    ///
    /// let mut vec = ArrayVec::<u8, 4>::new();
    /// vec.push(1);
    /// vec.push(2);
    /// vec.push(3);
    /// vec.push(4);
    ///
    /// assert!(vec.contains(&1));
    /// assert!(!vec.contains(&5));
    /// ```
    #[must_use]
    pub fn contains(&self, value: &T) -> bool {
        self.iter().any(|x| x == value)
    }
}

impl<T, const U: usize> ArrayVec<T, U>
where
    T: Copy + Ord,
{
    /// Sorts the `ArrayVec` in place.
    /// The sort is not stable, meaning that the relative order of elements that are equal is not
    /// preserved.
    ///
    /// # Examples
    ///
    /// ```
    /// use planck_noalloc::vec::ArrayVec;
    ///
    /// let mut vec = ArrayVec::<u8, 4>::new();
    /// vec.push(1);
    /// vec.push(2);
    /// vec.push(3);
    /// vec.push(4);
    ///
    /// vec.sort_unstable();
    ///
    /// assert_eq!(vec.as_slice(), &[1, 2, 3, 4]);
    /// ```
    pub fn sort_unstable(&mut self) {
        self.as_mut_slice().sort_unstable();
    }
}

impl<T, const N: usize> core::ops::Index<usize> for ArrayVec<T, N> {
    type Output = T;

    /// Returns a reference to the element at the given index.
    ///
    /// # Panics
    ///
    /// Panics if the index is out of bounds.
    ///
    /// # Examples
    ///
    /// ```
    /// use planck_noalloc::vec::ArrayVec;
    ///
    /// let mut vec = ArrayVec::<u8, 4>::new();
    /// vec.push(1);
    /// vec.push(2);
    /// vec.push(3);
    /// vec.push(4);
    ///
    /// assert_eq!(vec[0], 1);
    /// assert_eq!(vec[1], 2);
    /// assert_eq!(vec[2], 3);
    /// assert_eq!(vec[3], 4);
    /// ```
    fn index(&self, index: usize) -> &Self::Output {
        assert!(index < self.len, "index out of bounds");
        // SAFETY: Elements 0..self.len are initialized by the push invariant.
        unsafe { self.data[index].assume_init_ref() }
    }
}

impl<T, const N: usize> core::ops::IndexMut<usize> for ArrayVec<T, N> {
    /// Returns a mutable reference to the element at the given index.
    ///
    /// # Panics
    ///
    /// Panics if the index is out of bounds.
    ///
    /// # Examples
    ///
    /// ```
    /// use planck_noalloc::vec::ArrayVec;
    ///
    /// let mut vec = ArrayVec::<u8, 4>::new();
    /// vec.push(1);
    /// vec.push(2);
    /// vec.push(3);
    /// vec.push(4);
    ///
    /// assert_eq!(vec[0], 1);
    /// assert_eq!(vec[1], 2);
    /// assert_eq!(vec[2], 3);
    /// assert_eq!(vec[3], 4);
    ///
    /// vec[0] = 5;
    /// assert_eq!(vec[0], 5);
    /// ```
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        assert!(index < self.len, "index out of bounds");
        // SAFETY: Elements 0..self.len are initialized by the push invariant.
        unsafe { self.data[index].assume_init_mut() }
    }
}

impl<T, const N: usize> Drop for ArrayVec<T, N> {
    fn drop(&mut self) {
        // Drop all initialized elements.
        for i in 0..self.len {
            // SAFETY: Elements at indices 0..len are guaranteed to be initialized
            // by the ArrayVec invariant (push initializes, len tracks count).
            unsafe {
                self.data[i].assume_init_drop();
            }
        }
    }
}

impl<'a, T, const N: usize> IntoIterator for &'a ArrayVec<T, N> {
    type Item = &'a T;
    type IntoIter = core::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T, const N: usize> IntoIterator for &'a mut ArrayVec<T, N> {
    type Item = &'a mut T;
    type IntoIter = core::slice::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    extern crate std;
    use std::vec;
    use std::vec::Vec;

    use super::*;

    #[test]
    fn new_is_empty() {
        let vec = ArrayVec::<i32, 4>::new();
        assert!(vec.is_empty());
        assert_eq!(vec.len(), 0);
    }

    #[test]
    fn push_and_len() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(10);
        assert_eq!(vec.len(), 1);
        vec.push(20);
        assert_eq!(vec.len(), 2);
    }

    #[test]
    fn push_to_capacity() {
        let mut vec = ArrayVec::<i32, 3>::new();
        vec.push(1);
        vec.push(2);
        vec.push(3);
        assert_eq!(vec.len(), 3);
    }

    #[test]
    #[should_panic(expected = "ran out of capacity")]
    fn push_overflow_panics() {
        let mut vec = ArrayVec::<i32, 2>::new();
        vec.push(1);
        vec.push(2);
        vec.push(3); // should panic
    }

    #[test]
    fn try_push_ok_and_err() {
        let mut vec = ArrayVec::<i32, 2>::new();
        assert!(vec.try_push(1).is_ok());
        assert!(vec.try_push(2).is_ok());
        assert!(vec.try_push(3).is_err());
    }

    #[test]
    fn pop_returns_last() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(10);
        vec.push(20);
        assert_eq!(vec.pop(), Some(20));
        assert_eq!(vec.pop(), Some(10));
    }

    #[test]
    fn pop_empty_none() {
        let mut vec = ArrayVec::<i32, 4>::new();
        assert_eq!(vec.pop(), None);
    }

    #[test]
    fn clear_empties() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(1);
        vec.push(2);
        vec.clear();
        assert!(vec.is_empty());
        assert_eq!(vec.len(), 0);
    }

    #[test]
    fn swap_remove_middle() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(10);
        vec.push(20);
        vec.push(30);
        let removed = vec.swap_remove(1);
        assert_eq!(removed, 20);
        assert_eq!(vec.len(), 2);
        // 30 was swapped into index 1
        assert_eq!(vec.as_slice(), &[10, 30]);
    }

    #[test]
    fn swap_remove_first() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(10);
        vec.push(20);
        vec.push(30);
        let removed = vec.swap_remove(0);
        assert_eq!(removed, 10);
        assert_eq!(vec.as_slice(), &[30, 20]);
    }

    #[test]
    fn swap_remove_last() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(10);
        vec.push(20);
        let removed = vec.swap_remove(1);
        assert_eq!(removed, 20);
        assert_eq!(vec.as_slice(), &[10]);
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn swap_remove_out_of_bounds() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(1);
        vec.swap_remove(5);
    }

    #[test]
    fn as_slice_and_as_mut_slice() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(1);
        vec.push(2);
        vec.push(3);
        assert_eq!(vec.as_slice(), &[1, 2, 3]);
        vec.as_mut_slice()[1] = 99;
        assert_eq!(vec.as_slice(), &[1, 99, 3]);
    }

    #[test]
    fn index_and_index_mut() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(10);
        vec.push(20);
        assert_eq!(vec[0], 10);
        assert_eq!(vec[1], 20);
        vec[0] = 99;
        assert_eq!(vec[0], 99);
    }

    #[test]
    fn iter_collects() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(1);
        vec.push(2);
        vec.push(3);
        let collected: Vec<&i32> = vec.iter().collect();
        assert_eq!(collected, vec![&1, &2, &3]);
    }

    #[test]
    fn iter_mut_modifies() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(1);
        vec.push(2);
        vec.push(3);
        for v in vec.iter_mut() {
            *v *= 10;
        }
        assert_eq!(vec.as_slice(), &[10, 20, 30]);
    }

    #[test]
    fn reverse_elements() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(1);
        vec.push(2);
        vec.push(3);
        vec.push(4);
        vec.reverse();
        assert_eq!(vec.as_slice(), &[4, 3, 2, 1]);
    }

    #[test]
    fn reverse_single() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(42);
        vec.reverse();
        assert_eq!(vec.as_slice(), &[42]);
    }

    #[test]
    fn reverse_empty() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.reverse(); // should not panic
        assert!(vec.is_empty());
    }

    #[test]
    fn contains_element() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(10);
        vec.push(20);
        vec.push(30);
        assert!(vec.contains(&20));
        assert!(!vec.contains(&99));
    }

    #[test]
    fn sort_unstable_orders() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(3);
        vec.push(1);
        vec.push(4);
        vec.push(2);
        vec.sort_unstable();
        assert_eq!(vec.as_slice(), &[1, 2, 3, 4]);
    }

    #[test]
    fn sort_already_sorted() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(1);
        vec.push(2);
        vec.push(3);
        vec.sort_unstable();
        assert_eq!(vec.as_slice(), &[1, 2, 3]);
    }

    #[test]
    fn zero_capacity_vec() {
        let vec = ArrayVec::<i32, 0>::new();
        assert!(vec.is_empty());
        assert_eq!(vec.len(), 0);
    }

    #[test]
    fn zero_capacity_try_push_fails() {
        let mut vec = ArrayVec::<i32, 0>::new();
        assert!(vec.try_push(1).is_err());
    }

    #[test]
    fn insert_at_beginning() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(2);
        vec.push(3);
        vec.insert(0, 1);
        assert_eq!(vec.as_slice(), &[1, 2, 3]);
    }

    #[test]
    fn insert_at_middle() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(1);
        vec.push(3);
        vec.insert(1, 2);
        assert_eq!(vec.as_slice(), &[1, 2, 3]);
    }

    #[test]
    fn insert_at_end() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(1);
        vec.push(2);
        vec.insert(2, 3);
        assert_eq!(vec.as_slice(), &[1, 2, 3]);
    }

    #[test]
    fn insert_into_empty() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.insert(0, 42);
        assert_eq!(vec.as_slice(), &[42]);
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn insert_out_of_bounds() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(1);
        vec.insert(5, 2);
    }

    #[test]
    #[should_panic(expected = "ran out of capacity")]
    fn insert_full_panics() {
        let mut vec = ArrayVec::<i32, 2>::new();
        vec.push(1);
        vec.push(2);
        vec.insert(0, 3);
    }

    #[test]
    fn remove_first() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(1);
        vec.push(2);
        vec.push(3);
        let removed = vec.remove(0);
        assert_eq!(removed, 1);
        assert_eq!(vec.as_slice(), &[2, 3]);
    }

    #[test]
    fn remove_middle() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(1);
        vec.push(2);
        vec.push(3);
        let removed = vec.remove(1);
        assert_eq!(removed, 2);
        assert_eq!(vec.as_slice(), &[1, 3]);
    }

    #[test]
    fn remove_last() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(1);
        vec.push(2);
        vec.push(3);
        let removed = vec.remove(2);
        assert_eq!(removed, 3);
        assert_eq!(vec.as_slice(), &[1, 2]);
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn remove_out_of_bounds() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(1);
        vec.remove(5);
    }

    #[test]
    fn remove_only_element() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(42);
        let removed = vec.remove(0);
        assert_eq!(removed, 42);
        assert!(vec.is_empty());
    }

    #[test]
    fn last_empty() {
        let vec = ArrayVec::<i32, 4>::new();
        assert_eq!(vec.last(), None);
    }

    #[test]
    fn last_non_empty() {
        let mut vec = ArrayVec::<i32, 4>::new();
        vec.push(1);
        vec.push(2);
        assert_eq!(vec.last(), Some(&2));
    }

    #[test]
    fn is_full_check() {
        let mut vec = ArrayVec::<i32, 2>::new();
        assert!(!vec.is_full());
        vec.push(1);
        assert!(!vec.is_full());
        vec.push(2);
        assert!(vec.is_full());
    }

    #[test]
    fn insert_remove_round_trip() {
        let mut vec = ArrayVec::<i32, 8>::new();
        for i in 0..5 {
            vec.push(i * 10);
        }
        // [0, 10, 20, 30, 40]
        vec.insert(2, 15);
        // [0, 10, 15, 20, 30, 40]
        assert_eq!(vec.as_slice(), &[0, 10, 15, 20, 30, 40]);
        let removed = vec.remove(3);
        assert_eq!(removed, 20);
        // [0, 10, 15, 30, 40]
        assert_eq!(vec.as_slice(), &[0, 10, 15, 30, 40]);
    }
}
