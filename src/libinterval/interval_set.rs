// Copyright 2015 Pierre Talbot (IRCAM)

// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Closed and bounded generic interval set.
//!
//! It stores intervals in a set. The main advantage is the exact representation of an interval by allowing "holes". For example `[1..2] U [5..6]` is stored as `{[1..2], [5..6]}`. This structure is more space-efficient than a classic set collection (such as `BTreeSet`) if the data stored are mostly contiguous. Of course, it is less light-weight than [interval](../interval/index.html), but we keep the list of intervals as small as possible by merging overlapping intervals.
//!
//! ```rust
//! use crate::interval::interval_set::*;
//! use gcollections::ops::*;
//!
//! # fn main() {
//! let a = [(1,2), (6,10)].to_interval_set();
//! let b = [(3,5), (7,7)].to_interval_set();
//! let a_union_b = [(1,5), (6,10)].to_interval_set();
//! let a_intersect_b = [(7,7)].to_interval_set();
//!
//! assert_eq!(a.union(&b), a_union_b);
//! assert_eq!(a.intersection(&b), a_intersect_b);
//! # }
//! ```
//!
//! # See also
//! [interval](../interval/index.html)

use crate::interval::Interval;
use crate::interval::ToInterval;
use crate::ops::*;
use gcollections::ops::*;
use gcollections::*;
use serde::de::SeqAccess;
use serde::de::Visitor;
use serde::Deserialize;
use serde::Serialize;
use std::fmt;
use std::fmt::{Display, Error, Formatter};
use std::iter::{IntoIterator, Peekable};
use std::marker::PhantomData;
use std::ops::{Add, Mul, Sub};
use trilean::SKleene;

use num_traits::{Num, Zero};

#[derive(Debug, Clone)]
pub struct IntervalSet<Bound: Width> {
    intervals: Vec<Interval<Bound>>,
    size: Bound::Output,
}

impl<Bound> Serialize for IntervalSet<Bound>
where
    Bound: Width + Num + Serialize,
    Interval<Bound>: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if self.is_empty() {
            serializer.serialize_none()
        } else {
            serializer.collect_seq(&self.intervals)
        }
    }
}

impl<'de, Bound> Deserialize<'de> for IntervalSet<Bound>
where
    Bound: Width + Num + Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct IntervalSetVisitor<Bound> {
            marker: PhantomData<fn() -> Interval<Bound>>,
        }
        impl<Bound> IntervalSetVisitor<Bound> {
            fn new() -> Self {
                IntervalSetVisitor {
                    marker: PhantomData,
                }
            }
        }
        impl<'de, Bound> Visitor<'de> for IntervalSetVisitor<Bound>
        where
            Bound: Width + Deserialize<'de> + Num,
        {
            type Value = IntervalSet<Bound>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("sequence of intervals")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut intervals = Vec::new();
                if let Some(size) = seq.size_hint() {
                    intervals.reserve(size);
                }
                while let Some(interval) = seq.next_element::<Interval<Bound>>()? {
                    intervals.push(interval);
                }
                let mut interval_set = IntervalSet::empty();
                interval_set.extend(intervals);
                Ok(interval_set)
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(IntervalSet::empty())
            }
        }
        deserializer.deserialize_any(IntervalSetVisitor::new())
    }
}

impl<Bound: Width> IntervalKind for IntervalSet<Bound> {}

impl<Bound: Width> Collection for IntervalSet<Bound> {
    type Item = Bound;
}

impl<Bound: Width> IntoIterator for IntervalSet<Bound> {
    type Item = Interval<Bound>;
    type IntoIter = ::std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.intervals.into_iter()
    }
}

impl<'a, Bound: Width> IntoIterator for &'a IntervalSet<Bound> {
    type Item = &'a Interval<Bound>;
    type IntoIter = ::std::slice::Iter<'a, Interval<Bound>>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, Bound: Width> IntoIterator for &'a mut IntervalSet<Bound> {
    type Item = &'a mut Interval<Bound>;
    type IntoIter = ::std::slice::IterMut<'a, Interval<Bound>>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<Bound: Width> IntervalSet<Bound> {
    pub fn iter(&self) -> ::std::slice::Iter<Interval<Bound>> {
        self.intervals.iter()
    }

    pub fn iter_mut(&mut self) -> ::std::slice::IterMut<Interval<Bound>> {
        self.intervals.iter_mut()
    }
}

impl<Bound> IntervalSet<Bound>
where
    Bound: Width + Num,
{
    /// Counts the number of intervals contained in the set.
    /// ```
    /// # use interval::prelude::*;
    /// let single_interval_set = IntervalSet::new(0, 1);
    /// assert_eq!(single_interval_set.interval_count(), 1);
    /// let double_interval_set = [(3,5), (7,7)].to_interval_set();
    /// assert_eq!(double_interval_set.interval_count(), 2);
    /// let empty_interval_set = IntervalSet::<usize>::empty();
    /// assert_eq!(empty_interval_set.interval_count(), 0);
    /// ```
    pub fn interval_count(&self) -> usize {
        self.intervals.len()
    }

    fn from_interval(i: Interval<Bound>) -> IntervalSet<Bound> {
        let size = i.size().clone();
        IntervalSet {
            intervals: [i].to_vec(),
            size,
        }
    }

    fn front(&self) -> &Interval<Bound> {
        debug_assert!(
            !self.is_empty(),
            "Cannot access the first interval of an empty set."
        );
        &self.intervals[0]
    }

    fn back_idx(&self) -> usize {
        self.intervals.len() - 1
    }

    fn back(&self) -> &Interval<Bound> {
        debug_assert!(
            !self.is_empty(),
            "Cannot access the last interval of an empty set."
        );
        &self.intervals[self.back_idx()]
    }

    fn span(&self) -> Interval<Bound> {
        if self.is_empty() {
            Interval::empty()
        } else {
            self.span_slice(0, self.back_idx())
        }
    }

    fn span_slice(&self, left: usize, right: usize) -> Interval<Bound> {
        Interval::new(self.intervals[left].lower(), self.intervals[right].upper())
    }

    fn push(&mut self, x: Interval<Bound>) {
        debug_assert!(!x.is_empty(), "Cannot push empty interval.");
        debug_assert!(self.is_empty() || !joinable(self.back(), &x),
      "The intervals array must be ordered and intervals must not be joinable. For a safe push, use the union operation.");

        self.size = self.size.clone() + x.size();
        self.intervals.push(x);
    }

    fn pop(&mut self) -> Option<Interval<Bound>> {
        if let Some(x) = self.intervals.pop() {
            self.size = self.size.clone() - x.size();
            Some(x)
        } else {
            None
        }
    }

    fn join_or_push(&mut self, x: Interval<Bound>) {
        if self.is_empty() {
            self.push(x);
        } else {
            debug_assert!(!x.is_empty(), "Cannot push empty interval.");
            debug_assert!(self.back().lower() <= x.lower(),
        "This operation is only for pushing interval to the back of the array, possibly overlapping with the last element.");

            let joint = if joinable(self.back(), &x) {
                self.pop().unwrap().hull(&x)
            } else {
                x
            };
            self.push(joint);
        }
    }

    /// Expects the given iterable to be in-order - as we are iterating through
    /// it, each [`Interval<Bound>`] should belong on the upper end of the
    /// [`IntervalSet`]. Otherwise this method will panic.
    fn extend_at_back<I>(&mut self, iterable: I)
    where
        I: IntoIterator<Item = Interval<Bound>>,
        Bound: PartialOrd,
    {
        for interval in iterable {
            self.join_or_push(interval);
        }
    }

    fn find_interval_between(
        &self,
        value: &Bound,
        mut left: usize,
        mut right: usize,
    ) -> (usize, usize) {
        debug_assert!(left <= right);
        debug_assert!(right < self.intervals.len());
        debug_assert!(self.span_slice(left, right).contains(value));

        while left <= right {
            let mid_idx = left + (right - left) / 2;
            let mid = &self.intervals[mid_idx];
            if &mid.lower() > value {
                right = mid_idx - 1;
            } else if &mid.upper() < value {
                left = mid_idx + 1;
            } else {
                return (mid_idx, mid_idx);
            }
        }
        (right, left)
    }

    // Returns the indexes of the left and right interval of `value`.
    // If the value is outside `self`, returns None.
    // If the value is inside one of the interval, the indexes will be equal.
    fn find_interval(&self, value: &Bound) -> Option<(usize, usize)> {
        if !self.span().contains(value) {
            None
        } else {
            Some(self.find_interval_between(value, 0, self.back_idx()))
        }
    }

    fn for_all_pairs<F>(&self, other: &IntervalSet<Bound>, f: F) -> IntervalSet<Bound>
    where
        F: Fn(&Interval<Bound>, &Interval<Bound>) -> Interval<Bound>,
    {
        let mut res = IntervalSet::empty();
        for i in &self.intervals {
            for j in &other.intervals {
                res = res.union(&IntervalSet::from_interval(f(i, j)));
            }
        }
        res
    }

    // Precondition: `f` must not change the size of the interval.
    fn stable_map<F>(&self, f: F) -> IntervalSet<Bound>
    where
        F: Fn(&Interval<Bound>) -> Interval<Bound>,
    {
        IntervalSet {
            intervals: self.intervals.iter().map(f).collect(),
            size: self.size.clone(),
        }
    }

    fn map<F>(&self, f: F) -> IntervalSet<Bound>
    where
        F: Fn(&Interval<Bound>) -> Interval<Bound>,
    {
        self.intervals
            .iter()
            .fold(IntervalSet::empty(), |mut r, i| {
                r.push(f(i));
                r
            })
    }
}

fn joinable<Bound>(first: &Interval<Bound>, second: &Interval<Bound>) -> bool
where
    Bound: Width + Num,
{
    if first.upper() == Bound::max_value() {
        true
    } else {
        first.upper() + Bound::one() >= second.lower()
    }
}

impl<Bound> Extend<Interval<Bound>> for IntervalSet<Bound>
where
    Bound: Width + Num,
{
    /// Inserts additional intervals into an interval set.
    /// ```
    /// # use interval::prelude::*;
    /// let mut interval_set = IntervalSet::<u32>::empty();
    /// assert_eq!(interval_set, Vec::new().to_interval_set());
    /// interval_set.extend([Interval::new(2, 3), Interval::new(6, 7)]);
    /// // Now the set contains two disjoint intervals.
    /// assert_eq!(interval_set, [(2, 3), (6, 7)].to_interval_set());
    /// // Unify the intervals with the missing items to create a single interval.
    /// interval_set.extend([Interval::singleton(4), Interval::singleton(5)]);
    /// assert_eq!(interval_set, [(2, 7)].to_interval_set());
    /// ```
    fn extend<I>(&mut self, iterable: I)
    where
        I: IntoIterator<Item = Interval<Bound>>,
    {
        let mut intervals: Vec<_> = iterable.into_iter().map(|i| i.to_interval()).collect();
        intervals.sort_unstable_by_key(|i| i.lower());
        let mut set = IntervalSet::empty();
        set.extend_at_back(intervals);
        *self = self.union(&set);
    }
}

impl<Bound: Width + Num> Eq for IntervalSet<Bound> {}

impl<Bound> PartialEq<IntervalSet<Bound>> for IntervalSet<Bound>
where
    Bound: Width + Num,
{
    // Checks whether two interval sets are the same.
    /// ```
    /// # use interval::prelude::*;
    /// let single_interval = [(1, 5)].to_interval_set();
    /// let equivalent_interval = [(1, 2), (2, 5)].to_interval_set();
    /// assert_eq!(single_interval, equivalent_interval);
    /// ```
    /// Empty intervals are the same as each other, but not non-empty intervals.
    /// ```
    /// # use interval::prelude::*;
    /// assert_eq!(IntervalSet::<usize>::empty(), IntervalSet::<usize>::empty());
    /// assert_ne!(IntervalSet::empty(), [(2, 3), (8, 9)].to_interval_set());
    /// ```
    fn eq(&self, other: &IntervalSet<Bound>) -> bool {
        if self.size() != other.size() {
            false
        } else {
            self.intervals == other.intervals
        }
    }
}

impl<Bound> Range for IntervalSet<Bound>
where
    Bound: Width + Num,
{
    /// Constructs an interval set from a specified interval.
    /// ```
    /// # use interval::prelude::*;
    /// let interval = IntervalSet::new(2, 4);
    /// assert!(interval.contains(&2));
    /// assert!(interval.contains(&3));
    /// assert!(interval.contains(&4));
    ///
    /// assert!(!interval.contains(&1));
    /// assert!(!interval.contains(&5));
    /// ```
    /// Constructing an empty interval set is done via the [IntervalSet::empty()] method.
    /// ```
    /// # use interval::prelude::*;
    /// let empty_interval = IntervalSet::<u16>::empty();
    /// ```
    fn new(lb: Bound, ub: Bound) -> IntervalSet<Bound> {
        debug_assert!(lb <= ub, "Cannot build empty interval set with an invalid range. use crate::IntervalSet::empty().");
        let i = Interval::new(lb, ub);
        IntervalSet::from_interval(i)
    }
}

impl<Bound> Whole for IntervalSet<Bound>
where
    Bound: Width + Num,
{
    /// Constructs an interval set using the entire range an interval can represent.
    /// The bounds of the interval set are bounded in the same way as [`Interval::whole`].
    /// This is different for unsigned and signed integers:
    /// ```
    /// # use interval::prelude::*;
    /// let unsigned_interval = IntervalSet::<u8>::whole();
    /// assert_eq!(unsigned_interval, IntervalSet::new(0, 254));
    ///
    /// let signed_interval = IntervalSet::<i8>::whole();
    /// assert_eq!(signed_interval, IntervalSet::new(-127, 127));
    /// ```
    fn whole() -> IntervalSet<Bound> {
        let mut res = IntervalSet::empty();
        res.push(Interval::whole());
        res
    }
}

impl<Bound> Bounded for IntervalSet<Bound>
where
    Bound: Width + Num + PartialOrd,
{
    /// Returns the smallest value contained in an interval set.
    /// ```
    /// # use interval::prelude::*;
    /// assert_eq!([(8, 20)].to_interval_set().lower(), 8);
    /// assert_eq!([(-5, 11), (10, 30)].to_interval_set().lower(), -5);
    /// assert_eq!(IntervalSet::singleton(7).lower(), 7);
    /// ```
    /// However, this will panic on an empty interval.
    /// ```should_panic
    /// # use interval::prelude::*;
    /// IntervalSet::<u8>::empty().lower(); // panics!
    /// ```
    fn lower(&self) -> Bound {
        debug_assert!(
            !self.is_empty(),
            "Cannot access lower bound on empty interval."
        );
        self.front().lower()
    }

    /// Returns the largest value contained in an interval set.
    /// ```
    /// # use interval::prelude::*;
    /// assert_eq!([(8, 20)].to_interval_set().upper(), 20);
    /// assert_eq!([(-5, 11), (10, 30)].to_interval_set().upper(), 30);
    /// assert_eq!(IntervalSet::singleton(7).upper(), 7);
    /// ```
    /// However, this will panic on an empty interval.
    /// ```should_panic
    /// # use interval::prelude::*;
    /// IntervalSet::<u8>::empty().upper(); // panics!
    /// ```
    fn upper(&self) -> Bound {
        debug_assert!(
            !self.is_empty(),
            "Cannot access upper bound on empty interval."
        );
        self.back().upper()
    }
}

impl<Bound: Width + Num> Singleton for IntervalSet<Bound> {
    /// Constructs an interval set containing a single value.
    /// This set contains a single interval with the same upper and lower bounds.
    /// ```
    /// # use interval::prelude::*;
    /// let interval_set_5 = IntervalSet::singleton(5);
    /// assert_eq!(interval_set_5.size(), 1 as u32);
    /// assert_eq!(interval_set_5.lower(), interval_set_5.upper());
    /// assert!(interval_set_5.contains(&5));
    /// assert!(!interval_set_5.is_empty());
    /// ```
    fn singleton(x: Bound) -> IntervalSet<Bound> {
        IntervalSet::new(x.clone(), x)
    }
}

impl<Bound: Width + Num> Empty for IntervalSet<Bound> {
    /// Constructs an empty interval set.
    /// The type needs to be specified or inferred as `Interval` is parametrized by its input.
    /// ```
    /// # use interval::prelude::*;
    /// assert_eq!(IntervalSet::<i32>::empty().size(), 0);
    /// assert_eq!(IntervalSet::<u32>::empty().size(), 0);
    /// assert!(IntervalSet::<u16>::empty().is_empty());
    /// ```
    fn empty() -> IntervalSet<Bound> {
        IntervalSet {
            intervals: Vec::new(),
            size: <<Bound as Width>::Output>::zero(),
        }
    }
}

/// `IsSingleton` and `IsEmpty` are defined automatically in `gcollections`.
impl<Bound: Width + Num> Cardinality for IntervalSet<Bound> {
    type Size = <Bound as Width>::Output;

    /// Calculates the size of an interval set (the number of integers it contains).
    /// This includes the endpoints of all intervals.
    /// The size of the interval set is unsigned, but has the same number of bits as the interval bounds.
    /// ```
    /// # use interval::prelude::*;
    /// assert_eq!(IntervalSet::<i32>::singleton(8).size(), 1 as u32);
    /// assert_eq!(IntervalSet::<usize>::new(1, 10).size(), 10 as usize);
    /// // Default is to use i32.
    /// assert_eq!([(3, 5), (8, 9)].to_interval_set().size(), (3 + 2) as u32);
    /// assert_eq!(IntervalSet::<i64>::empty().size(), 0);
    /// // Doesn't overflow:
    /// assert_eq!(IntervalSet::<usize>::whole().size(), usize::max_value());
    /// ```
    fn size(&self) -> <Bound as Width>::Output {
        self.size.clone()
    }
}

impl<Bound: Width + Num> Contains for IntervalSet<Bound> {
    /// Calculates whether an interval contains a value.
    /// ```
    /// # use interval::prelude::*;
    /// let interval_set = [(3, 5), (8, 9)].to_interval_set();
    /// assert!(interval_set.contains(&3));
    /// assert!(interval_set.contains(&4));
    /// assert!(interval_set.contains(&5));
    /// assert!(interval_set.contains(&8));
    /// assert!(interval_set.contains(&9));
    ///
    /// assert!(!interval_set.contains(&1));
    /// assert!(!interval_set.contains(&2));
    /// assert!(!interval_set.contains(&6));
    /// assert!(!interval_set.contains(&7));
    /// assert!(!interval_set.contains(&10));
    /// ```
    fn contains(&self, value: &Bound) -> bool {
        if let Some((left, right)) = self.find_interval(value) {
            left == right
        } else {
            false
        }
    }
}

fn advance_one<I, F, Item>(a: &mut Peekable<I>, b: &mut Peekable<I>, choose: F) -> Item
where
    I: Iterator<Item = Item>,
    F: Fn(&Item, &Item) -> bool,
    Item: Bounded,
{
    static NON_EMPTY_PRECONDITION: &str =
        "`advance_one` expects both interval iterators to be non_empty.";
    let who_advance = {
        let i = a.peek().expect(NON_EMPTY_PRECONDITION);
        let j = b.peek().expect(NON_EMPTY_PRECONDITION);
        choose(i, j)
    };
    let to_advance = if who_advance { a } else { b };
    to_advance.next().unwrap()
}

fn advance_lower<I, Item, B>(a: &mut Peekable<I>, b: &mut Peekable<I>) -> Item
where
    I: Iterator<Item = Item>,
    Item: Bounded + Collection<Item = B>,
    B: Ord,
{
    advance_one(a, b, |i, j| i.lower() < j.lower())
}

// Advance the one with the lower upper bound.
fn advance_lub<I, Item, B>(a: &mut Peekable<I>, b: &mut Peekable<I>) -> Item
where
    I: Iterator<Item = Item>,
    Item: Bounded + Collection<Item = B>,
    B: Ord,
{
    advance_one(a, b, |i, j| i.upper() < j.upper())
}

fn from_lower_iterator<I, Bound>(a: &mut Peekable<I>, b: &mut Peekable<I>) -> IntervalSet<Bound>
where
    I: Iterator<Item = Interval<Bound>>,
    Bound: Width + Num,
{
    if a.peek().is_none() || b.peek().is_none() {
        IntervalSet::empty()
    } else {
        let first = advance_lower(a, b);
        IntervalSet::from_interval(first)
    }
}

impl<Bound> Union<Bound> for IntervalSet<Bound>
where
    Bound: Width + Num + Clone,
{
    type Output = IntervalSet<Bound>;

    /// Adds a value to the interval set.
    /// The value does not have to below to an existing interval,
    /// but if it does, those intervals will be merged.
    /// ```
    /// # use interval::prelude::*;
    /// let interval_set = [(1, 4), (6, 7)].to_interval_set();
    /// assert_eq!(interval_set.union(&10), [(1, 4), (6, 7), (10, 10)].to_interval_set());
    ///
    /// assert_eq!(interval_set.union(&3), interval_set);
    /// assert_eq!(interval_set.union(&6), interval_set);
    ///
    /// assert_eq!(interval_set.union(&0), [(0, 4), (6, 7)].to_interval_set());
    /// assert_eq!(interval_set.union(&8), [(1, 4), (6, 8)].to_interval_set());
    ///
    /// assert_eq!(interval_set.union(&5), [(1, 7)].to_interval_set());
    /// ```
    fn union(&self, rhs: &Bound) -> IntervalSet<Bound> {
        self.union(&IntervalSet::singleton(rhs.clone()))
    }
}

impl<Bound: Width + Num> Union for IntervalSet<Bound> {
    type Output = IntervalSet<Bound>;

    /// Calculates the union of two interval sets.
    /// This returns an interval set containing all the values in the two that were unified.
    /// ```
    /// # use interval::prelude::*;
    /// let a = [(1, 3), (8, 8), (10, 11)].to_interval_set();
    /// let b = [(2, 5), (7, 8), (12, 15)].to_interval_set();
    /// assert_eq!(a.union(&b), [(1, 5), (7, 8), (10, 15)].to_interval_set());
    /// ```
    fn union(&self, rhs: &IntervalSet<Bound>) -> IntervalSet<Bound> {
        let a = &mut self.intervals.iter().cloned().peekable();
        let b = &mut rhs.intervals.iter().cloned().peekable();
        let mut res = from_lower_iterator(a, b);
        while a.peek().is_some() && b.peek().is_some() {
            let lower = advance_lower(a, b);
            res.join_or_push(lower);
        }
        res.extend_at_back(a);
        res.extend_at_back(b);
        res
    }
}

// Returns `false` when one of the iterator is consumed.
// Iterators are not consumed if the intervals are already overlapping.
fn advance_to_first_overlapping<I, Item, B>(a: &mut Peekable<I>, b: &mut Peekable<I>) -> bool
where
    I: Iterator<Item = Item>,
    Item: Bounded + Overlap + Collection<Item = B>,
    B: Ord,
{
    while a.peek().is_some() && b.peek().is_some() {
        let overlapping = {
            let i = a.peek().unwrap();
            let j = b.peek().unwrap();
            i.overlap(j)
        };
        if overlapping {
            return true;
        } else {
            advance_lower(a, b);
        }
    }
    false
}

impl<Bound> Intersection<Bound> for IntervalSet<Bound>
where
    Bound: Width + Num + Clone,
{
    type Output = IntervalSet<Bound>;

    /// Calculates whether a value is contained in an interval.
    /// Returns the value as an interval set if it is in the interval
    /// and an empty interval set if not.
    /// ```
    /// # use interval::prelude::*;
    /// let interval_set = [(1, 4), (6, 7)].to_interval_set();
    /// assert_eq!(interval_set.intersection(&10), IntervalSet::empty());
    ///
    /// assert_eq!(interval_set.intersection(&3), IntervalSet::singleton(3));
    /// assert_eq!(interval_set.intersection(&6), IntervalSet::singleton(6));
    /// ```
    fn intersection(&self, rhs: &Bound) -> IntervalSet<Bound> {
        self.intersection(&IntervalSet::singleton(rhs.clone()))
    }
}

impl<Bound: Width + Num> Intersection for IntervalSet<Bound> {
    type Output = IntervalSet<Bound>;

    /// Calculates the intersection of two interval sets.
    /// This returns an interval set containing all the values in the both.
    /// ```
    /// # use interval::prelude::*;
    /// let a = [(1, 3), (8, 8), (10, 11)].to_interval_set();
    /// let b = [(2, 5), (7, 8), (12, 15)].to_interval_set();
    /// assert_eq!(a.intersection(&b), [(2, 3), (8, 8)].to_interval_set());
    /// ```
    fn intersection(&self, rhs: &IntervalSet<Bound>) -> IntervalSet<Bound> {
        let a = &mut self.intervals.iter().cloned().peekable();
        let b = &mut rhs.intervals.iter().cloned().peekable();
        let mut res = IntervalSet::empty();
        while advance_to_first_overlapping(a, b) {
            {
                let i = a.peek().unwrap();
                let j = b.peek().unwrap();
                res.push(i.intersection(j));
            }
            advance_lub(a, b); // advance the one with the lowest upper bound.
        }
        res
    }
}

fn push_left_complement<Bound: Width + Num>(x: &Interval<Bound>, res: &mut IntervalSet<Bound>) {
    let min = <Bound as Width>::min_value();
    if x.lower() != min {
        res.push(Interval::new(min, x.lower() - Bound::one()));
    }
}

fn push_right_complement<Bound: Width + Num>(x: &Interval<Bound>, res: &mut IntervalSet<Bound>) {
    let max = <Bound as Width>::max_value();
    if x.upper() != max {
        res.push(Interval::new(x.upper() + Bound::one(), max));
    }
}

impl<Bound: Width + Num> Complement for IntervalSet<Bound> {
    /// Calculates all values that are excluded from the interval set.
    /// Positive and negative infinity are represented with `Interval::whole().lower()` and `Interval::whole().upper()`;
    /// ```
    /// # use interval::prelude::*;
    /// assert_eq!(IntervalSet::<i64>::empty().complement(), IntervalSet::whole());
    ///
    /// let neg_inf = IntervalSet::<i32>::whole().lower();
    /// let pos_inf = IntervalSet::<i32>::whole().upper();
    /// assert_eq!(IntervalSet::singleton(5).complement(), [(neg_inf, 4), (6, pos_inf)].to_interval_set());
    /// assert_eq!([(2, 5), (8, 10)].to_interval_set().complement(), [(neg_inf, 1), (6, 7), (11, pos_inf)].to_interval_set());
    /// ```
    fn complement(&self) -> IntervalSet<Bound> {
        let mut res = IntervalSet::empty();
        if self.is_empty() {
            res.push(Interval::whole());
        } else {
            let one = Bound::one();
            push_left_complement(self.front(), &mut res);
            for i in 1..self.intervals.len() {
                let current = &self.intervals[i];
                let previous = &self.intervals[i - 1];
                res.push(Interval::new(
                    previous.upper() + one.clone(),
                    current.lower() - one.clone(),
                ));
            }
            push_right_complement(self.back(), &mut res);
        }
        res
    }
}

impl<Bound> Difference<Bound> for IntervalSet<Bound>
where
    Bound: Width + Num + Clone,
{
    type Output = IntervalSet<Bound>;

    /// Calculates the interval set after removing a single value.
    /// This returns the original interval set if the value is not in the interval set.
    /// ```
    /// # use interval::prelude::*;
    /// let interval_set = [(1, 3), (5, 9)].to_interval_set();
    /// assert_eq!(interval_set.difference(&4), interval_set);
    /// assert_eq!(interval_set.difference(&5), [(1, 3), (6, 9)].to_interval_set());
    /// assert_eq!(interval_set.difference(&7), [(1, 3), (5, 6), (8, 9)].to_interval_set());
    /// ```
    fn difference(&self, rhs: &Bound) -> IntervalSet<Bound> {
        self.difference(&IntervalSet::singleton(rhs.clone()))
    }
}

impl<Bound: Width + Num> Difference for IntervalSet<Bound> {
    type Output = IntervalSet<Bound>;

    /// Calculates the interval set containing all items in the left set,
    /// but not in the right set.
    /// The difference is not symmetric.
    /// ```
    /// # use interval::prelude::*;
    /// let a = [(1, 3), (8, 8), (10, 11)].to_interval_set();
    /// let b = [(2, 5), (7, 8), (12, 15)].to_interval_set();
    /// assert_eq!(a.difference(&b), [(1, 1), (10, 11)].to_interval_set());
    /// assert_eq!(b.difference(&a), [(4, 5), (7, 7), (12, 15)].to_interval_set());
    /// ```
    fn difference(&self, rhs: &IntervalSet<Bound>) -> IntervalSet<Bound> {
        self.intersection(&rhs.complement())
    }
}

impl<Bound> SymmetricDifference<Bound> for IntervalSet<Bound>
where
    Bound: Width + Num + Clone,
{
    type Output = IntervalSet<Bound>;

    /// If the value is in the interval set, this return the interval set without the value.
    /// If the value is not in the interval set, this returns the interval set extended with the value.
    /// ```
    /// # use interval::prelude::*;
    /// let interval_set = [(1, 3), (5, 9)].to_interval_set();
    /// assert_eq!(interval_set.symmetric_difference(&4), [(1, 9)].to_interval_set());
    /// assert_eq!(interval_set.symmetric_difference(&4), interval_set.union(&4));

    /// assert_eq!(interval_set.symmetric_difference(&5), [(1, 3), (6, 9)].to_interval_set());
    /// assert_eq!(interval_set.symmetric_difference(&5), interval_set.difference(&5));
    /// ```
    fn symmetric_difference(&self, rhs: &Bound) -> IntervalSet<Bound> {
        self.symmetric_difference(&IntervalSet::singleton(rhs.clone()))
    }
}

impl<Bound: Width + Num> SymmetricDifference for IntervalSet<Bound> {
    type Output = IntervalSet<Bound>;

    /// Calculates the interval set containing that are in either the left set or the right set,
    /// but not both.
    /// ```
    /// # use interval::prelude::*;
    /// let a = [(1, 3), (8, 8), (10, 11)].to_interval_set();
    /// let b = [(2, 5), (7, 8), (12, 15)].to_interval_set();
    /// let symmetric_difference = [(1, 1), (4, 5), (7, 7), (10, 15)].to_interval_set();
    /// assert_eq!(a.symmetric_difference(&b), symmetric_difference);
    /// assert_eq!(b.symmetric_difference(&a), symmetric_difference);
    ///
    /// assert_eq!(IntervalSet::union(&a.difference(&b), &b.difference(&a)), symmetric_difference);
    /// ```
    fn symmetric_difference(&self, rhs: &IntervalSet<Bound>) -> IntervalSet<Bound> {
        let union = self.union(rhs);
        let intersection = self.intersection(rhs);
        union.difference(&intersection)
    }
}

impl<Bound: Width + Num> Overlap for IntervalSet<Bound> {
    /// Calculates whether two interval contain any shared values.
    /// ```
    /// # use interval::prelude::*;
    /// let a = [(1, 3), (7, 8)].to_interval_set();
    /// let b = [(4, 6)].to_interval_set();
    /// assert!(!a.overlap(&b));
    /// assert!(!b.overlap(&a));
    ///
    /// let a = [(1, 3)].to_interval_set();
    /// let b = [(3, 4), (8, 10)].to_interval_set();
    /// assert!(a.overlap(&b));
    /// assert!(b.overlap(&a));
    /// ```
    fn overlap(&self, rhs: &IntervalSet<Bound>) -> bool {
        let a = &mut self.intervals.iter().cloned().peekable();
        let b = &mut rhs.intervals.iter().cloned().peekable();
        advance_to_first_overlapping(a, b)
    }
}

impl<Bound: Width + Num> Overlap<Bound> for IntervalSet<Bound> {
    /// Calculates whether a value is included in the interval set.
    /// This returns the same result as the [`IntervalSet::contains`]
    /// ```
    /// # use interval::prelude::*;
    /// let interval_set = [(3, 5), (8, 9)].to_interval_set();
    /// assert!(interval_set.overlap(&3));
    /// assert!(interval_set.overlap(&8));
    /// assert!(interval_set.overlap(&9));
    ///
    /// assert!(!interval_set.overlap(&1));
    /// assert!(!interval_set.overlap(&7));
    /// assert!(!interval_set.overlap(&10));
    /// ```
    fn overlap(&self, value: &Bound) -> bool {
        if let Some((l, u)) = self.find_interval(value) {
            l == u
        } else {
            false
        }
    }
}

impl<Bound: Width + Num> Overlap<Optional<Bound>> for IntervalSet<Bound> {
    /// Calculates whether an optional value is included in the interval set.
    /// If the optional empty, this returns false.
    /// This returns the same result as the [`IntervalSet::contains`]
    /// ```
    /// # use interval::prelude::*;
    /// let interval_set = [(3, 5), (8, 9)].to_interval_set();
    /// assert!(interval_set.overlap(&Optional::singleton(3)));
    /// assert!(interval_set.overlap(&Optional::singleton(9)));
    ///
    /// assert!(!interval_set.overlap(&Optional::singleton(1)));
    /// assert!(!interval_set.overlap(&Optional::singleton(10)));
    ///
    /// assert!(!interval_set.overlap(&Optional::empty()));
    /// ```
    fn overlap(&self, value: &Optional<Bound>) -> bool {
        value.as_ref().map_or(false, |b| self.overlap(b))
    }
}

macro_rules! primitive_interval_set_overlap
{
  ( $( $source:ty ),* ) =>
  {$(
    impl Overlap<IntervalSet<$source>> for $source {
      #[doc = concat!(
        r#"
        Calculates whether a value is included in an interval set.
        ```
        # use interval::prelude::*;
        let interval_set: IntervalSet<"#, stringify!($source), r#"> = [(3, 5), (8, 9)].to_interval_set();
        assert!((3 as "#, stringify!($source), r#").overlap(&interval_set));
        assert!((8 as "#, stringify!($source), r#").overlap(&interval_set));
        assert!((9 as "#, stringify!($source), r#").overlap(&interval_set));
        ///
        assert!(!(1 as "#, stringify!($source), r#").overlap(&interval_set));
        assert!(!(7 as "#, stringify!($source), r#").overlap(&interval_set));
        assert!(!(10 as "#, stringify!($source), r#").overlap(&interval_set));
        ```
        "#
      )]
      fn overlap(&self, other: &IntervalSet<$source>) -> bool {
        other.overlap(self)
      }
    }
  )*}
}

primitive_interval_set_overlap!(i8, u8, i16, u16, i32, u32, i64, u64, isize, usize);

impl<Bound: Width + Num> Disjoint for IntervalSet<Bound> {
    /// Calculates whether two interval do *not* contain any shared values.
    /// ```
    /// # use interval::prelude::*;
    /// let a = [(1, 3), (7, 8)].to_interval_set();
    /// let b = [(4, 6)].to_interval_set();
    /// assert!(a.is_disjoint(&b));
    /// assert!(b.is_disjoint(&a));
    ///
    /// let a = [(1, 3)].to_interval_set();
    /// let b = [(3, 4), (8, 10)].to_interval_set();
    /// assert!(!a.is_disjoint(&b));
    /// assert!(!b.is_disjoint(&a));
    /// ```
    fn is_disjoint(&self, rhs: &IntervalSet<Bound>) -> bool {
        !self.overlap(rhs)
    }
}

impl<Bound: Width + Num> ShrinkLeft for IntervalSet<Bound>
where
    <Bound as Width>::Output: Clone,
{
    /// Updates the lower bound of an interval set to be greater than or equal to a value.
    /// ```
    /// # use interval::prelude::*;
    /// let interval_set = [(4, 5), (8, 8)].to_interval_set();
    /// assert_eq!(interval_set.shrink_left(2), interval_set);
    /// assert_eq!(interval_set.shrink_left(4), interval_set);
    /// assert_eq!(interval_set.shrink_left(5), [(5, 5), (8, 8)].to_interval_set());
    /// assert_eq!(interval_set.shrink_left(7), IntervalSet::singleton(8));
    /// assert_eq!(interval_set.shrink_left(8), IntervalSet::singleton(8));
    /// assert_eq!(interval_set.shrink_left(9), IntervalSet::empty());
    /// ```
    fn shrink_left(&self, lb: Bound) -> IntervalSet<Bound> {
        if let Some((left, _)) = self.find_interval(&lb) {
            let mut res = IntervalSet::empty();
            if self.intervals[left].upper() >= lb {
                res.push(Interval::new(lb, self.intervals[left].upper()));
            }
            for i in (left + 1)..self.intervals.len() {
                res.push(self.intervals[i].clone());
            }
            res
        } else if self.is_empty() || lb > self.back().upper() {
            IntervalSet::empty()
        } else {
            self.clone()
        }
    }
}

impl<Bound: Width + Num> ShrinkRight for IntervalSet<Bound>
where
    <Bound as Width>::Output: Clone,
{
    /// Updates the upper bound of an interval set to be less than or equal to a value.
    /// ```
    /// # use interval::prelude::*;
    /// let interval_set = [(3, 3), (7, 8)].to_interval_set();
    /// assert_eq!(interval_set.shrink_right(9), interval_set);
    /// assert_eq!(interval_set.shrink_right(8), interval_set);
    /// assert_eq!(interval_set.shrink_right(7), [(3, 3), (7, 7)].to_interval_set());
    /// assert_eq!(interval_set.shrink_right(6), IntervalSet::singleton(3));
    /// assert_eq!(interval_set.shrink_right(3), IntervalSet::singleton(3));
    /// assert_eq!(interval_set.shrink_right(2), IntervalSet::empty());
    /// ```
    fn shrink_right(&self, ub: Bound) -> IntervalSet<Bound> {
        if let Some((_, right)) = self.find_interval(&ub) {
            let mut res = IntervalSet::empty();
            for i in 0..right {
                res.push(self.intervals[i].clone());
            }
            if self.intervals[right].lower() <= ub {
                res.push(Interval::new(self.intervals[right].lower(), ub));
            }
            res
        } else if self.is_empty() || ub < self.front().lower() {
            IntervalSet::empty()
        } else {
            self.clone()
        }
    }
}

impl<Bound: Width + Num> Subset for IntervalSet<Bound> {
    /// Calculates whether one interval set is contained in another.
    /// The empty interval set is a subset of everything.
    /// ```
    /// # use interval::prelude::*;
    /// let interval_set = [(3, 3), (7, 8)].to_interval_set();
    /// assert!(interval_set.is_subset(&[(3, 8)].to_interval_set()));
    /// assert!(interval_set.is_subset(&[(3, 4), (7, 9)].to_interval_set()));
    /// assert!(interval_set.is_subset(&interval_set));
    ///
    /// assert!(!interval_set.is_subset(&[(3, 3)].to_interval_set()));
    /// assert!(!interval_set.is_subset(&[(7, 9)].to_interval_set()));
    /// assert!(!interval_set.is_subset(&[(3, 3), (8, 9)].to_interval_set()));
    ///
    /// assert!(IntervalSet::<usize>::empty().is_subset(&IntervalSet::empty()));
    /// assert!(IntervalSet::empty().is_subset(&interval_set));
    /// ```
    fn is_subset(&self, other: &IntervalSet<Bound>) -> bool {
        if self.is_empty() {
            true
        } else if self.size() > other.size() || !self.span().is_subset(&other.span()) {
            false
        } else {
            let mut left = 0;
            let right = other.intervals.len() - 1;
            for interval in &self.intervals {
                let (l, r) = other.find_interval_between(&interval.lower(), left, right);
                if l == r && interval.is_subset(&other.intervals[l]) {
                    left = l;
                } else {
                    return false;
                }
            }
            true
        }
    }
}

impl<Bound: Width + Num> ProperSubset for IntervalSet<Bound> {
    /// Calculates whether one interval set is contained in another,
    /// but they are not equal.
    /// The empty interval set is a proper subset of everything, except itself.
    /// ```
    /// # use interval::prelude::*;
    /// let interval_set = [(3, 3), (7, 8)].to_interval_set();
    /// assert!(interval_set.is_proper_subset(&[(3, 8)].to_interval_set()));
    /// assert!(interval_set.is_proper_subset(&[(3, 4), (7, 9)].to_interval_set()));
    ///
    /// assert!(!interval_set.is_proper_subset(&interval_set));
    /// assert!(!interval_set.is_proper_subset(&[(3, 3)].to_interval_set()));
    /// assert!(!interval_set.is_proper_subset(&[(7, 9)].to_interval_set()));
    /// assert!(!interval_set.is_proper_subset(&[(3, 3), (8, 9)].to_interval_set()));
    ///
    /// assert!(IntervalSet::empty().is_proper_subset(&interval_set));
    /// assert!(!IntervalSet::<usize>::empty().is_proper_subset(&IntervalSet::empty()));
    /// ```
    fn is_proper_subset(&self, other: &IntervalSet<Bound>) -> bool {
        self.is_subset(other) && self.size() != other.size()
    }
}

forward_all_binop!(impl<Bound: +Num+Width> Add for IntervalSet<Bound>, add);

impl<'a, 'b, Bound: Num + Width> Add<&'b IntervalSet<Bound>> for &'a IntervalSet<Bound> {
    type Output = IntervalSet<Bound>;

    /// Calculates all values that could result in the addition of two items from each interval set.
    /// ```
    /// # use interval::prelude::*;
    /// let a = [(1, 2), (5, 6)].to_interval_set();
    /// let b = [(1, 1), (4, 5)].to_interval_set();
    /// assert_eq!(a + b, [(2, 3), (5, 7), (9, 11)].to_interval_set());
    /// ```
    /// This method preserves empty interval sets.
    /// ```
    /// # use interval::prelude::*;
    /// let a = [(1, 1), (4, 5)].to_interval_set();
    /// let b = IntervalSet::empty();
    /// assert!((a + b).is_empty());
    /// ```
    fn add(self, other: &IntervalSet<Bound>) -> IntervalSet<Bound> {
        self.for_all_pairs(other, |i, j| i + j)
    }
}

forward_all_binop!(impl<Bound: +Num+Width+Clone> Add for IntervalSet<Bound>, add, Bound);

impl<'a, 'b, Bound: Num + Width + Clone> Add<&'b Bound> for &'a IntervalSet<Bound> {
    type Output = IntervalSet<Bound>;

    /// Adds a constant to an interval set.
    /// ```
    /// # use interval::prelude::*;
    /// assert_eq!([(3, 3), (7, 8)].to_interval_set() + 2, [(5, 5), (9, 10)].to_interval_set());
    /// ```
    /// This method preserves empty interval sets.
    /// ```
    /// # use interval::prelude::*;
    /// assert!((IntervalSet::empty() + 4).is_empty());
    /// ```
    /// It is not possible to add an interval set to a constant.
    /// ```compile_fail
    /// # use interval::prelude::*;
    /// let _ = 4 + IntervalSet::new(5, 9); // doesn't compile
    /// ```
    fn add(self, other: &Bound) -> IntervalSet<Bound> {
        self.stable_map(|x| x + other.clone())
    }
}

forward_all_binop!(impl<Bound: +Num+Width> Sub for IntervalSet<Bound>, sub);

impl<'a, 'b, Bound: Num + Width> Sub<&'b IntervalSet<Bound>> for &'a IntervalSet<Bound> {
    type Output = IntervalSet<Bound>;

    fn sub(self, other: &IntervalSet<Bound>) -> IntervalSet<Bound> {
        self.for_all_pairs(other, |i, j| i - j)
    }
}

forward_all_binop!(impl<Bound: +Num+Width+Clone> Sub for IntervalSet<Bound>, sub, Bound);

impl<'a, 'b, Bound: Num + Width + Clone> Sub<&'b Bound> for &'a IntervalSet<Bound> {
    type Output = IntervalSet<Bound>;

    /// Subtracts a constant from an interval set.
    /// ```
    /// # use interval::prelude::*;
    /// assert_eq!([(3, 3), (7, 8)].to_interval_set() - 2, [(1, 1), (5, 6)].to_interval_set());
    /// ```
    /// This method preserves empty interval sets.
    /// ```
    /// # use interval::prelude::*;
    /// assert!((IntervalSet::empty() - 4).is_empty());
    /// ```
    /// It is not possible to substract an interval set from a constant.
    /// ```compile_fail
    /// # use interval::prelude::*;
    /// let _ = 10 - IntervalSet::new(5, 9); // doesn't compile
    /// ```
    fn sub(self, other: &Bound) -> IntervalSet<Bound> {
        self.stable_map(|x| x - other.clone())
    }
}

forward_all_binop!(impl<Bound: +Num+Width> Mul for IntervalSet<Bound>, mul);

impl<'a, 'b, Bound: Num + Width> Mul<&'b IntervalSet<Bound>> for &'a IntervalSet<Bound> {
    type Output = IntervalSet<Bound>;

    /// Calculates all values that could result in the multiplication of two items from each interval set.
    /// Caution: the resulting interval set is an over-approxmation for the same reason as [`Interval::mul`](../interval/struct.Interval.html#method.mul-3).
    /// ```
    /// # use interval::prelude::*;
    /// let a = [(1, 2), (5, 6)].to_interval_set();
    /// let b = [(0, 0), (3, 4)].to_interval_set();
    /// assert_eq!(a * b, [(0, 0), (3, 8), (15, 24)].to_interval_set());
    /// ```
    /// This method preserves empty interval sets.
    /// ```
    /// # use interval::prelude::*;
    /// assert!((IntervalSet::empty() * [(0, 0), (3, 4)].to_interval_set()).is_empty());
    /// ```
    fn mul(self, other: &IntervalSet<Bound>) -> IntervalSet<Bound> {
        self.for_all_pairs(other, |i, j| i * j)
    }
}

forward_all_binop!(impl<Bound: +Num+Width+Clone> Mul for IntervalSet<Bound>, mul, Bound);

impl<'a, 'b, Bound: Num + Width + Clone> Mul<&'b Bound> for &'a IntervalSet<Bound> {
    type Output = IntervalSet<Bound>;

    /// Multiplies an interval set by a constant.
    /// Caution: the resulting interval set is an over-approxmation for the same reason as [`Interval::mul`](../interval/struct.Interval.html#method.mul-7).
    /// ```
    /// # use interval::prelude::*;
    /// assert_eq!([(1, 2), (5, 6)].to_interval_set() * 2, [(2, 4), (10, 12)].to_interval_set());
    /// ```
    /// This method preserves empty interval sets.
    /// ```
    /// # use interval::prelude::*;
    /// assert!((IntervalSet::empty() * 11).is_empty());
    /// ```
    /// It is not possible to multiply a constant by an interval set.
    /// ```compile_fail
    /// # use interval::prelude::*;
    /// let _ = 4 * IntervalSet::new(5, 9); // doesn't compile
    /// ```
    fn mul(self, other: &Bound) -> IntervalSet<Bound> {
        if self.is_empty() {
            IntervalSet::empty()
        } else if other == &Bound::zero() {
            IntervalSet::singleton(Bound::zero())
        } else if other == &Bound::one() {
            self.clone()
        } else {
            self.map(|i| i * other.clone())
        }
    }
}

pub trait ToIntervalSet<Bound>
where
    Bound: Width,
{
    /// Converts a value to an interval set.
    /// For example,
    /// ```
    /// # use interval::prelude::*;
    /// assert_eq!((3, 4).to_interval_set(), IntervalSet::new(3, 4));
    /// assert_eq!([(2, 5), (7, 8)].to_interval_set(), IntervalSet::union(&IntervalSet::new(2, 5), &IntervalSet::new(7, 8)));
    /// ```
    fn to_interval_set(self) -> IntervalSet<Bound>;
}

impl<Bound: Width + Num> ToIntervalSet<Bound> for (Bound, Bound) {
    /// Converts a tuple to an interval set using the first element as the lower bound
    /// and second element as the upper bound.
    /// ```
    /// # use interval::prelude::*;
    /// assert_eq!((2, 6).to_interval_set(), IntervalSet::new(2, 6));
    /// ```
    /// The first and second elements need the same type.
    /// ```compile_fail
    /// # use interval::prelude::*;
    /// let _ = (8 as u8, 9 as i8).to_interval_set(); // doesn't compile
    /// ```
    fn to_interval_set(self) -> IntervalSet<Bound> {
        [self].to_interval_set()
    }
}

impl<Bound> ToIntervalSet<Bound> for Vec<(Bound, Bound)>
where
    Bound: Width + Num,
{
    /// Converts a vector of intervals to an interval set.
    /// ```
    /// # use interval::prelude::*;
    /// assert_eq!(vec![(2, 5)].to_interval_set().interval_count(), 1);
    /// assert_eq!(vec![(1, 5), (11, 20)].to_interval_set().interval_count(), 2);
    /// assert!(Vec::<(usize, usize)>::new().to_interval_set().is_empty());
    /// ```
    fn to_interval_set(self) -> IntervalSet<Bound> {
        let mut intervals = IntervalSet::empty();
        let mut to_add: Vec<_> = self.into_iter().map(|i| i.to_interval()).collect();
        to_add.sort_unstable_by_key(|i| i.lower());
        intervals.extend_at_back(to_add);
        intervals
    }
}

impl<Bound> ToIntervalSet<Bound> for &[(Bound, Bound)]
where
    Bound: Width + Num + Copy,
{
    /// Converts an array to an interval set.
    /// ```
    /// # use interval::prelude::*;
    /// assert_eq!([(2, 5)].to_interval_set().interval_count(), 1);
    /// assert_eq!([(1, 5), (11, 20)].to_interval_set().interval_count(), 2);
    /// assert!(<&[(usize, usize)]>::default().to_interval_set().is_empty());
    /// ```
    fn to_interval_set(self) -> IntervalSet<Bound> {
        self.to_vec().to_interval_set()
    }
}

impl<Bound, const N: usize> ToIntervalSet<Bound> for [(Bound, Bound); N]
where
    Bound: Width + Num + Clone,
{
    /// Converts a fixed-length array to an interval set.
    /// ```
    /// # use interval::prelude::*;
    /// assert_eq!([(2, 5)].to_interval_set().interval_count(), 1);
    /// assert_eq!([(1, 5), (11, 20)].to_interval_set().interval_count(), 2);
    /// assert!(([] as [(usize, usize); 0]).to_interval_set().is_empty());
    /// ```
    fn to_interval_set(self) -> IntervalSet<Bound> {
        self.to_vec().to_interval_set()
    }
}

impl<Bound: Display + Width + Num> Display for IntervalSet<Bound>
where
    <Bound as Width>::Output: Display,
{
    /// Formats an interval set.
    /// Empty interval sets are displayed as the empty set "{}".
    /// Single intervals are displayed as the isolated interval.
    /// Combined intervals are displayed as a sorted set of intervals.
    /// See [`Interval::fmt`](../interval/struct.Interval.html#method.fmt-1) for more detail on how intervals are formatted.
    /// ```
    /// # use interval::prelude::*;
    /// assert_eq!(format!("{}", [(3, 5)].to_interval_set()), "[3..5]");
    /// assert_eq!(format!("{}", [(4, 4), (8, 9)].to_interval_set()), "{[4..4][8..9]}");
    /// assert_eq!(format!("{}", IntervalSet::<u32>::empty()), "{}");
    /// ```
    fn fmt(&self, formatter: &mut Formatter) -> Result<(), Error> {
        if self.intervals.len() == 1 {
            self.intervals[0].fmt(formatter)
        } else {
            formatter.write_str("{")?;
            for interval in &self.intervals {
                formatter.write_fmt(format_args!("{}", interval))?;
            }
            formatter.write_str("}")
        }
    }
}

impl<Bound> Join for IntervalSet<Bound>
where
    Bound: Width + Num,
{
    fn join(self, other: IntervalSet<Bound>) -> IntervalSet<Bound> {
        self.intersection(&other)
    }
}

impl<Bound> Meet for IntervalSet<Bound>
where
    Bound: Width + Num,
{
    fn meet(self, other: IntervalSet<Bound>) -> IntervalSet<Bound> {
        self.union(&other)
    }
}

impl<Bound> Entailment for IntervalSet<Bound>
where
    Bound: Width + Num,
{
    fn entail(&self, other: &IntervalSet<Bound>) -> SKleene {
        if self.is_subset(other) {
            SKleene::True
        } else if other.is_subset(self) {
            SKleene::False
        } else {
            SKleene::Unknown
        }
    }
}

impl<Bound> Top for IntervalSet<Bound>
where
    Bound: Width + Num,
{
    fn top() -> IntervalSet<Bound> {
        IntervalSet::empty()
    }
}

impl<Bound> Bot for IntervalSet<Bound>
where
    Bound: Width + Num,
{
    fn bot() -> IntervalSet<Bound> {
        IntervalSet::whole()
    }
}

#[allow(non_upper_case_globals)]
#[cfg(test)]
mod tests {
    use serde_test::{assert_tokens, Token};

    use super::*;

    const extend_example: [(i32, i32); 2] = [(11, 33), (-55, -44)];

    fn test_inside_outside(is: IntervalSet<i32>, inside: Vec<i32>, outside: Vec<i32>) {
        for i in &inside {
            assert!(
                is.contains(i),
                "{} is not contained inside {}, but it should.",
                i,
                is
            );
        }
        for i in &outside {
            assert!(
                !is.contains(i),
                "{} is contained inside {}, but it should not.",
                i,
                is
            );
        }
    }

    // precondition: `intervals` must be a valid intern representation of the interval set.
    fn make_interval_set(intervals: Vec<(i32, i32)>) -> IntervalSet<i32> {
        intervals.to_interval_set()
    }

    fn test_result(test_id: String, result: &IntervalSet<i32>, expected: &IntervalSet<i32>) {
        assert!(
            result.intervals == expected.intervals,
            "{} | {} is different from the expected value: {}.",
            test_id,
            result,
            expected
        );
    }

    fn test_binary_op_sym<F>(
        test_id: String,
        a: Vec<(i32, i32)>,
        b: Vec<(i32, i32)>,
        op: F,
        expected: Vec<(i32, i32)>,
    ) where
        F: Fn(&IntervalSet<i32>, &IntervalSet<i32>) -> IntervalSet<i32>,
    {
        test_binary_op(
            test_id.clone(),
            a.clone(),
            b.clone(),
            |i, j| op(i, j),
            expected.clone(),
        );
        test_binary_op(test_id, b, a, op, expected);
    }

    fn test_binary_op<F>(
        test_id: String,
        a: Vec<(i32, i32)>,
        b: Vec<(i32, i32)>,
        op: F,
        expected: Vec<(i32, i32)>,
    ) where
        F: Fn(&IntervalSet<i32>, &IntervalSet<i32>) -> IntervalSet<i32>,
    {
        println!("Info: {}.", test_id);
        let a = make_interval_set(a);
        let b = make_interval_set(b);
        let expected = make_interval_set(expected);
        test_result(test_id, &op(&a, &b), &expected);
    }

    fn test_binary_value_op<F>(
        test_id: String,
        a: Vec<(i32, i32)>,
        b: i32,
        op: F,
        expected: Vec<(i32, i32)>,
    ) where
        F: Fn(&IntervalSet<i32>, i32) -> IntervalSet<i32>,
    {
        println!("Info: {}.", test_id);
        let a = make_interval_set(a);
        let expected = make_interval_set(expected);
        test_result(test_id, &op(&a, b), &expected);
    }

    fn test_binary_bool_op_sym<F>(
        test_id: String,
        a: Vec<(i32, i32)>,
        b: Vec<(i32, i32)>,
        op: F,
        expected: bool,
    ) where
        F: Fn(&IntervalSet<i32>, &IntervalSet<i32>) -> bool,
    {
        test_binary_bool_op(
            test_id.clone(),
            a.clone(),
            b.clone(),
            |i, j| op(i, j),
            expected,
        );
        test_binary_bool_op(test_id, b, a, op, expected);
    }

    fn test_binary_bool_op<F>(
        test_id: String,
        a: Vec<(i32, i32)>,
        b: Vec<(i32, i32)>,
        op: F,
        expected: bool,
    ) where
        F: Fn(&IntervalSet<i32>, &IntervalSet<i32>) -> bool,
    {
        println!("Info: {}.", test_id);
        let a = make_interval_set(a);
        let b = make_interval_set(b);
        assert_eq!(op(&a, &b), expected);
    }

    fn test_binary_value_bool_op<V, F>(
        test_id: String,
        a: Vec<(i32, i32)>,
        b: V,
        op: F,
        expected: bool,
    ) where
        F: Fn(&IntervalSet<i32>, &V) -> bool,
    {
        println!("Info: {}.", test_id);
        let a = make_interval_set(a);
        assert_eq!(op(&a, &b), expected);
    }

    fn test_op<F>(test_id: String, a: Vec<(i32, i32)>, op: F, expected: Vec<(i32, i32)>)
    where
        F: Fn(&IntervalSet<i32>) -> IntervalSet<i32>,
    {
        println!("Info: {}.", test_id);
        let a = make_interval_set(a);
        let expected = make_interval_set(expected);
        let result = op(&a);
        test_result(test_id, &result, &expected);
    }

    #[test]
    fn test_contains() {
        let cases = vec![
            (vec![], vec![], vec![-2, -1, 0, 1, 2]),
            (vec![(1, 2)], vec![1, 2], vec![-1, 0, 3, 4]),
            (
                vec![(1, 2), (7, 9)],
                vec![1, 2, 7, 8, 9],
                vec![-1, 0, 3, 4, 5, 6, 10, 11],
            ),
            (
                vec![(1, 2), (4, 5), (7, 9)],
                vec![1, 2, 4, 5, 7, 8, 9],
                vec![-1, 0, 3, 6, 10, 11],
            ),
        ];

        for (is, inside, outside) in cases {
            let is = make_interval_set(is);
            test_inside_outside(is, inside, outside);
        }
    }

    #[test]
    fn test_complement() {
        let min = <i32 as Width>::min_value();
        let max = <i32 as Width>::max_value();

        let cases = vec![
            (1, vec![], vec![(min, max)]),
            (2, vec![(min, max)], vec![]),
            (3, vec![(0, 0)], vec![(min, -1), (1, max)]),
            (4, vec![(-5, 5)], vec![(min, -6), (6, max)]),
            (5, vec![(-5, -1), (1, 5)], vec![(min, -6), (0, 0), (6, max)]),
            (6, vec![(min, -1), (1, 5)], vec![(0, 0), (6, max)]),
            (7, vec![(-5, -1), (1, max)], vec![(min, -6), (0, 0)]),
            (8, vec![(min, -1), (1, max)], vec![(0, 0)]),
            (
                9,
                vec![(-5, -3), (0, 1), (3, 5)],
                vec![(min, -6), (-2, -1), (2, 2), (6, max)],
            ),
        ];

        for (id, a, expected) in cases {
            test_op(
                format!("test #{} of complement", id),
                a.clone(),
                |x| x.complement(),
                expected,
            );
            test_op(
                format!("test #{} of complement(complement)", id),
                a.clone(),
                |x| x.complement().complement(),
                a,
            );
        }
    }

    #[test]
    fn test_union() {
        // Note: the first number is the test id, so it should be easy to identify which test has failed.
        // The two first vectors are the operands and the expected result is last.
        let sym_cases = vec![
            // identity tests
            (1, vec![], vec![], vec![]),
            (2, vec![], vec![(1, 2)], vec![(1, 2)]),
            (3, vec![], vec![(1, 2), (7, 9)], vec![(1, 2), (7, 9)]),
            (4, vec![(1, 2), (7, 9)], vec![(1, 2)], vec![(1, 2), (7, 9)]),
            (
                5,
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (7, 9)],
            ),
            // front tests
            (
                6,
                vec![(-3, -1)],
                vec![(1, 2), (7, 9)],
                vec![(-3, -1), (1, 2), (7, 9)],
            ),
            (
                7,
                vec![(-3, 0)],
                vec![(1, 2), (7, 9)],
                vec![(-3, 2), (7, 9)],
            ),
            (
                8,
                vec![(-3, 1)],
                vec![(1, 2), (7, 9)],
                vec![(-3, 2), (7, 9)],
            ),
            // middle tests
            (9, vec![(2, 7)], vec![(1, 2), (7, 9)], vec![(1, 9)]),
            (10, vec![(3, 7)], vec![(1, 2), (7, 9)], vec![(1, 9)]),
            (
                11,
                vec![(4, 5)],
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (4, 5), (7, 9)],
            ),
            (12, vec![(2, 8)], vec![(1, 2), (7, 9)], vec![(1, 9)]),
            (13, vec![(2, 6)], vec![(1, 2), (7, 9)], vec![(1, 9)]),
            (14, vec![(3, 6)], vec![(1, 2), (7, 9)], vec![(1, 9)]),
            // back tests
            (15, vec![(8, 9)], vec![(1, 2), (7, 9)], vec![(1, 2), (7, 9)]),
            (
                16,
                vec![(8, 10)],
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (7, 10)],
            ),
            (
                17,
                vec![(9, 10)],
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (7, 10)],
            ),
            (
                18,
                vec![(6, 10)],
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (6, 10)],
            ),
            (
                19,
                vec![(10, 11)],
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (7, 11)],
            ),
            (
                20,
                vec![(11, 12)],
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (7, 9), (11, 12)],
            ),
            // mixed tests
            (
                21,
                vec![(-3, -1), (4, 5), (11, 12)],
                vec![(1, 2), (7, 9)],
                vec![(-3, -1), (1, 2), (4, 5), (7, 9), (11, 12)],
            ),
            (
                22,
                vec![(-3, 0), (3, 6), (10, 11)],
                vec![(1, 2), (7, 9)],
                vec![(-3, 11)],
            ),
            (
                23,
                vec![(-3, 1), (3, 7), (9, 11)],
                vec![(1, 2), (7, 9)],
                vec![(-3, 11)],
            ),
            (
                24,
                vec![(-3, 5), (7, 11)],
                vec![(1, 2), (7, 9)],
                vec![(-3, 5), (7, 11)],
            ),
            (
                25,
                vec![(-3, 5), (7, 8), (12, 12)],
                vec![(1, 2), (7, 9)],
                vec![(-3, 5), (7, 9), (12, 12)],
            ),
            // englobing tests
            (26, vec![(-1, 11)], vec![(1, 2), (7, 9)], vec![(-1, 11)]),
        ];

        for (id, a, b, expected) in sym_cases {
            test_binary_op_sym(
                format!("test #{} of union", id),
                a,
                b,
                |x, y| x.union(y),
                expected,
            );
        }
    }

    #[test]
    fn test_intersection() {
        // Note: the first number is the test id, so it should be easy to identify which test has failed.
        // The two first vectors are the operands and the expected result is last.
        let sym_cases = vec![
            // identity tests
            (1, vec![], vec![], vec![]),
            (2, vec![], vec![(1, 2)], vec![]),
            (3, vec![], vec![(1, 2), (7, 9)], vec![]),
            (4, vec![(1, 2), (7, 9)], vec![(1, 2)], vec![(1, 2)]),
            (
                5,
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (7, 9)],
            ),
            // front tests
            (6, vec![(-3, -1)], vec![(1, 2), (7, 9)], vec![]),
            (7, vec![(-3, 0)], vec![(1, 2), (7, 9)], vec![]),
            (8, vec![(-3, 1)], vec![(1, 2), (7, 9)], vec![(1, 1)]),
            // middle tests
            (9, vec![(2, 7)], vec![(1, 2), (7, 9)], vec![(2, 2), (7, 7)]),
            (10, vec![(3, 7)], vec![(1, 2), (7, 9)], vec![(7, 7)]),
            (11, vec![(4, 5)], vec![(1, 2), (7, 9)], vec![]),
            (12, vec![(2, 8)], vec![(1, 2), (7, 9)], vec![(2, 2), (7, 8)]),
            (13, vec![(2, 6)], vec![(1, 2), (7, 9)], vec![(2, 2)]),
            (14, vec![(3, 6)], vec![(1, 2), (7, 9)], vec![]),
            // back tests
            (15, vec![(8, 9)], vec![(1, 2), (7, 9)], vec![(8, 9)]),
            (16, vec![(8, 10)], vec![(1, 2), (7, 9)], vec![(8, 9)]),
            (17, vec![(9, 10)], vec![(1, 2), (7, 9)], vec![(9, 9)]),
            (18, vec![(6, 10)], vec![(1, 2), (7, 9)], vec![(7, 9)]),
            (19, vec![(10, 11)], vec![(1, 2), (7, 9)], vec![]),
            (20, vec![(11, 12)], vec![(1, 2), (7, 9)], vec![]),
            // mixed tests
            (
                21,
                vec![(-3, -1), (4, 5), (11, 12)],
                vec![(1, 2), (7, 9)],
                vec![],
            ),
            (
                22,
                vec![(-3, 0), (3, 6), (10, 11)],
                vec![(1, 2), (7, 9)],
                vec![],
            ),
            (
                23,
                vec![(-3, 1), (3, 7), (9, 11)],
                vec![(1, 2), (7, 9)],
                vec![(1, 1), (7, 7), (9, 9)],
            ),
            (
                24,
                vec![(-3, 5), (7, 11)],
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (7, 9)],
            ),
            (
                25,
                vec![(-3, 5), (7, 8), (12, 12)],
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (7, 8)],
            ),
            // englobing tests
            (
                26,
                vec![(-1, 11)],
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (7, 9)],
            ),
        ];

        for (id, a, b, expected) in sym_cases {
            test_binary_op_sym(
                format!("test #{} of intersection", id),
                a,
                b,
                |x, y| x.intersection(y),
                expected,
            );
        }
    }

    #[test]
    fn test_to_interval_set() {
        // This example should not panic, and should yield the correct result.
        let intervals = vec![(3, 8), (2, 5)].to_interval_set();
        assert_eq!(intervals.interval_count(), 1);
        assert_eq!(intervals.lower(), 2);
        assert_eq!(intervals.upper(), 8);
    }

    #[test]
    #[should_panic(
        expected = "This operation is only for pushing interval to the back of the array, possibly overlapping with the last element."
    )]
    fn test_extend_back() {
        // Calling extend_at_back with unordered input should panic.
        let mut set = IntervalSet::empty();
        let intervals = extend_example.map(|i| i.to_interval());
        set.extend_at_back(intervals);
        assert_eq!(set.interval_count(), 2);
    }

    #[test]
    fn test_extend_empty() {
        // Calling extend with unordered input should not panic.
        let mut set = IntervalSet::empty();
        let intervals = extend_example.map(|i| i.to_interval());
        set.extend(intervals);
        assert_eq!(set.interval_count(), 2);
    }

    #[test]
    fn test_extend_non_empty() {
        // Extending an IntervalSet with intervals that belong at the start or
        // the middle of the set should not panic.
        let mut intervals = vec![(10, 15), (20, 30)].to_interval_set();
        let at_start = vec![(0, 5).to_interval()];
        intervals.extend(at_start);
        let in_middle = vec![(17, 18).to_interval()];
        intervals.extend(in_middle);

        assert_eq!(intervals.interval_count(), 4);
        assert_eq!(intervals.lower(), 0);
        assert_eq!(intervals.upper(), 30);
    }

    #[test]
    fn test_difference() {
        // Note: the first number is the test id, so it should be easy to identify which test has failed.
        // The two first vectors are the operands and the two last are expected results (the first
        // for a.difference(b) and the second for b.difference(a)).
        let cases = vec![
            // identity tests
            (1, vec![], vec![], vec![], vec![]),
            (2, vec![], vec![(1, 2)], vec![], vec![(1, 2)]),
            (
                3,
                vec![],
                vec![(1, 2), (7, 9)],
                vec![],
                vec![(1, 2), (7, 9)],
            ),
            (4, vec![(1, 2), (7, 9)], vec![(1, 2)], vec![(7, 9)], vec![]),
            (
                5,
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (7, 9)],
                vec![],
                vec![],
            ),
            // front tests
            (
                6,
                vec![(-3, -1)],
                vec![(1, 2), (7, 9)],
                vec![(-3, -1)],
                vec![(1, 2), (7, 9)],
            ),
            (
                7,
                vec![(-3, 0)],
                vec![(1, 2), (7, 9)],
                vec![(-3, 0)],
                vec![(1, 2), (7, 9)],
            ),
            (
                8,
                vec![(-3, 1)],
                vec![(1, 2), (7, 9)],
                vec![(-3, 0)],
                vec![(2, 2), (7, 9)],
            ),
            // middle tests
            (
                9,
                vec![(2, 7)],
                vec![(1, 2), (7, 9)],
                vec![(3, 6)],
                vec![(1, 1), (8, 9)],
            ),
            (
                10,
                vec![(3, 7)],
                vec![(1, 2), (7, 9)],
                vec![(3, 6)],
                vec![(1, 2), (8, 9)],
            ),
            (
                11,
                vec![(4, 5)],
                vec![(1, 2), (7, 9)],
                vec![(4, 5)],
                vec![(1, 2), (7, 9)],
            ),
            (
                12,
                vec![(2, 8)],
                vec![(1, 2), (7, 9)],
                vec![(3, 6)],
                vec![(1, 1), (9, 9)],
            ),
            (
                13,
                vec![(2, 6)],
                vec![(1, 2), (7, 9)],
                vec![(3, 6)],
                vec![(1, 1), (7, 9)],
            ),
            (
                14,
                vec![(3, 6)],
                vec![(1, 2), (7, 9)],
                vec![(3, 6)],
                vec![(1, 2), (7, 9)],
            ),
            // back tests
            (
                15,
                vec![(8, 9)],
                vec![(1, 2), (7, 9)],
                vec![],
                vec![(1, 2), (7, 7)],
            ),
            (
                16,
                vec![(8, 10)],
                vec![(1, 2), (7, 9)],
                vec![(10, 10)],
                vec![(1, 2), (7, 7)],
            ),
            (
                17,
                vec![(9, 10)],
                vec![(1, 2), (7, 9)],
                vec![(10, 10)],
                vec![(1, 2), (7, 8)],
            ),
            (
                18,
                vec![(6, 10)],
                vec![(1, 2), (7, 9)],
                vec![(6, 6), (10, 10)],
                vec![(1, 2)],
            ),
            (
                19,
                vec![(10, 11)],
                vec![(1, 2), (7, 9)],
                vec![(10, 11)],
                vec![(1, 2), (7, 9)],
            ),
            (
                20,
                vec![(11, 12)],
                vec![(1, 2), (7, 9)],
                vec![(11, 12)],
                vec![(1, 2), (7, 9)],
            ),
            // mixed tests
            (
                21,
                vec![(-3, -1), (4, 5), (11, 12)],
                vec![(1, 2), (7, 9)],
                vec![(-3, -1), (4, 5), (11, 12)],
                vec![(1, 2), (7, 9)],
            ),
            (
                22,
                vec![(-3, 0), (3, 6), (10, 11)],
                vec![(1, 2), (7, 9)],
                vec![(-3, 0), (3, 6), (10, 11)],
                vec![(1, 2), (7, 9)],
            ),
            (
                23,
                vec![(-3, 1), (3, 7), (9, 11)],
                vec![(1, 2), (7, 9)],
                vec![(-3, 0), (3, 6), (10, 11)],
                vec![(2, 2), (8, 8)],
            ),
            (
                24,
                vec![(-3, 5), (7, 11)],
                vec![(1, 2), (7, 9)],
                vec![(-3, 0), (3, 5), (10, 11)],
                vec![],
            ),
            (
                25,
                vec![(-3, 5), (7, 8), (12, 12)],
                vec![(1, 2), (7, 9)],
                vec![(-3, 0), (3, 5), (12, 12)],
                vec![(9, 9)],
            ),
            // englobing tests
            (
                26,
                vec![(-1, 11)],
                vec![(1, 2), (7, 9)],
                vec![(-1, 0), (3, 6), (10, 11)],
                vec![],
            ),
        ];

        for (id, a, b, expected, expected_sym) in cases {
            test_binary_op(
                format!("test #{} of difference", id),
                a.clone(),
                b.clone(),
                |x, y| x.difference(y),
                expected,
            );
            test_binary_op(
                format!("test #{} of difference", id),
                b,
                a,
                |x, y| x.difference(y),
                expected_sym,
            );
        }
    }

    #[test]
    fn test_symmetric_difference() {
        // Note: the first number is the test id, so it should be easy to identify which test has failed.
        // The two first vectors are the operands and the expected result is last.
        let sym_cases = vec![
            // identity tests
            (1, vec![], vec![], vec![]),
            (2, vec![], vec![(1, 2)], vec![(1, 2)]),
            (3, vec![], vec![(1, 2), (7, 9)], vec![(1, 2), (7, 9)]),
            (4, vec![(1, 2), (7, 9)], vec![(1, 2)], vec![(7, 9)]),
            (5, vec![(1, 2), (7, 9)], vec![(1, 2), (7, 9)], vec![]),
            // front tests
            (
                6,
                vec![(-3, -1)],
                vec![(1, 2), (7, 9)],
                vec![(-3, -1), (1, 2), (7, 9)],
            ),
            (
                7,
                vec![(-3, 0)],
                vec![(1, 2), (7, 9)],
                vec![(-3, 2), (7, 9)],
            ),
            (
                8,
                vec![(-3, 1)],
                vec![(1, 2), (7, 9)],
                vec![(-3, 0), (2, 2), (7, 9)],
            ),
            // middle tests
            (
                9,
                vec![(2, 7)],
                vec![(1, 2), (7, 9)],
                vec![(1, 1), (3, 6), (8, 9)],
            ),
            (10, vec![(3, 7)], vec![(1, 2), (7, 9)], vec![(1, 6), (8, 9)]),
            (
                11,
                vec![(4, 5)],
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (4, 5), (7, 9)],
            ),
            (
                12,
                vec![(2, 8)],
                vec![(1, 2), (7, 9)],
                vec![(1, 1), (3, 6), (9, 9)],
            ),
            (13, vec![(2, 6)], vec![(1, 2), (7, 9)], vec![(1, 1), (3, 9)]),
            (14, vec![(3, 6)], vec![(1, 2), (7, 9)], vec![(1, 9)]),
            // back tests
            (15, vec![(8, 9)], vec![(1, 2), (7, 9)], vec![(1, 2), (7, 7)]),
            (
                16,
                vec![(8, 10)],
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (7, 7), (10, 10)],
            ),
            (
                17,
                vec![(9, 10)],
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (7, 8), (10, 10)],
            ),
            (
                18,
                vec![(6, 10)],
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (6, 6), (10, 10)],
            ),
            (
                19,
                vec![(10, 11)],
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (7, 11)],
            ),
            (
                20,
                vec![(11, 12)],
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (7, 9), (11, 12)],
            ),
            // mixed tests
            (
                21,
                vec![(-3, -1), (4, 5), (11, 12)],
                vec![(1, 2), (7, 9)],
                vec![(-3, -1), (1, 2), (4, 5), (7, 9), (11, 12)],
            ),
            (
                22,
                vec![(-3, 0), (3, 6), (10, 11)],
                vec![(1, 2), (7, 9)],
                vec![(-3, 11)],
            ),
            (
                23,
                vec![(-3, 1), (3, 7), (9, 11)],
                vec![(1, 2), (7, 9)],
                vec![(-3, 0), (2, 6), (8, 8), (10, 11)],
            ),
            (
                24,
                vec![(-3, 5), (7, 11)],
                vec![(1, 2), (7, 9)],
                vec![(-3, 0), (3, 5), (10, 11)],
            ),
            (
                25,
                vec![(-3, 5), (7, 8), (12, 12)],
                vec![(1, 2), (7, 9)],
                vec![(-3, 0), (3, 5), (9, 9), (12, 12)],
            ),
            // englobing tests
            (
                26,
                vec![(-1, 11)],
                vec![(1, 2), (7, 9)],
                vec![(-1, 0), (3, 6), (10, 11)],
            ),
        ];

        for (id, a, b, expected) in sym_cases {
            test_binary_op_sym(
                format!("test #{} of symmetric difference", id),
                a,
                b,
                |x, y| x.symmetric_difference(y),
                expected,
            );
        }
    }

    #[test]
    fn test_overlap_and_is_disjoint() {
        // Note: the first number is the test id, so it should be easy to identify which test has failed.
        // The two first vectors are the operands and the expected result is last.
        let sym_cases = vec![
            // identity tests
            (1, vec![], vec![], false),
            (2, vec![], vec![(1, 2)], false),
            (3, vec![], vec![(1, 2), (7, 9)], false),
            (4, vec![(1, 2), (7, 9)], vec![(1, 2)], true),
            (5, vec![(1, 2), (7, 9)], vec![(1, 2), (7, 9)], true),
            // front tests
            (6, vec![(-3, -1)], vec![(1, 2), (7, 9)], false),
            (7, vec![(-3, 0)], vec![(1, 2), (7, 9)], false),
            (8, vec![(-3, 1)], vec![(1, 2), (7, 9)], true),
            // middle tests
            (9, vec![(2, 7)], vec![(1, 2), (7, 9)], true),
            (10, vec![(3, 7)], vec![(1, 2), (7, 9)], true),
            (11, vec![(4, 5)], vec![(1, 2), (7, 9)], false),
            (12, vec![(2, 8)], vec![(1, 2), (7, 9)], true),
            (13, vec![(2, 6)], vec![(1, 2), (7, 9)], true),
            (14, vec![(3, 6)], vec![(1, 2), (7, 9)], false),
            // back tests
            (15, vec![(8, 9)], vec![(1, 2), (7, 9)], true),
            (16, vec![(8, 10)], vec![(1, 2), (7, 9)], true),
            (17, vec![(9, 10)], vec![(1, 2), (7, 9)], true),
            (18, vec![(6, 10)], vec![(1, 2), (7, 9)], true),
            (19, vec![(10, 11)], vec![(1, 2), (7, 9)], false),
            (20, vec![(11, 12)], vec![(1, 2), (7, 9)], false),
            // mixed tests
            (
                21,
                vec![(-3, -1), (4, 5), (11, 12)],
                vec![(1, 2), (7, 9)],
                false,
            ),
            (
                22,
                vec![(-3, 0), (3, 6), (10, 11)],
                vec![(1, 2), (7, 9)],
                false,
            ),
            (
                23,
                vec![(-3, 1), (3, 7), (9, 11)],
                vec![(1, 2), (7, 9)],
                true,
            ),
            (24, vec![(-3, 5), (7, 11)], vec![(1, 2), (7, 9)], true),
            (
                25,
                vec![(-3, 5), (7, 8), (12, 12)],
                vec![(1, 2), (7, 9)],
                true,
            ),
            // englobing tests
            (26, vec![(-1, 11)], vec![(1, 2), (7, 9)], true),
        ];

        for (id, a, b, expected) in sym_cases {
            test_binary_bool_op_sym(
                format!("test #{} of overlap", id),
                a.clone(),
                b.clone(),
                |x, y| x.overlap(y),
                expected,
            );
            test_binary_bool_op_sym(
                format!("test #{} of is_disjoint", id),
                a,
                b,
                |x, y| x.is_disjoint(y),
                !expected,
            );
        }
    }

    fn overlap_cases() -> Vec<(u32, Vec<(i32, i32)>, i32, bool)> {
        vec![
            (1, vec![], 0, false),
            (2, vec![(1, 2)], 0, false),
            (3, vec![(1, 2)], 1, true),
            (4, vec![(1, 2)], 2, true),
            (5, vec![(1, 2)], 3, false),
            (6, vec![(1, 3), (5, 7)], 2, true),
            (7, vec![(1, 3), (5, 7)], 6, true),
            (8, vec![(1, 3), (5, 7)], 4, false),
            (9, vec![(1, 3), (5, 7)], 0, false),
            (10, vec![(1, 3), (5, 7)], 8, false),
        ]
    }

    #[test]
    fn test_overlap_bound() {
        let cases = overlap_cases();

        for (id, a, b, expected) in cases {
            test_binary_value_bool_op(
                format!("test #{} of overlap_bound", id),
                a.clone(),
                b,
                |x, y| x.overlap(y),
                expected,
            );
            test_binary_value_bool_op(
                format!("test #{} of overlap_bound", id),
                a,
                b,
                |x, y| y.overlap(x),
                expected,
            );
        }
    }

    #[test]
    fn test_overlap_option() {
        let mut cases: Vec<(u32, Vec<(i32, i32)>, Optional<i32>, bool)> = overlap_cases()
            .into_iter()
            .map(|(id, a, b, e)| (id, a, Optional::singleton(b), e))
            .collect();
        cases.extend(
            vec![
                (11, vec![], Optional::empty(), false),
                (12, vec![(1, 2)], Optional::empty(), false),
                (13, vec![(1, 3), (5, 7)], Optional::empty(), false),
            ]
            .into_iter(),
        );

        for (id, a, b, expected) in cases {
            test_binary_value_bool_op(
                format!("test #{} of overlap_option", id),
                a.clone(),
                b,
                |x, y| x.overlap(y),
                expected,
            );
        }
    }

    #[test]
    fn test_shrink() {
        let min = <i32 as Width>::min_value();
        let max = <i32 as Width>::max_value();

        // Second and third args are the test value.
        // The fourth is the result of a shrink_left and the fifth of a shrink_right.
        let cases = vec![
            (1, vec![], 0, vec![], vec![]),
            (2, vec![(min, max)], 0, vec![(0, max)], vec![(min, 0)]),
            (3, vec![(0, 0)], 0, vec![(0, 0)], vec![(0, 0)]),
            (4, vec![(0, 0)], -1, vec![(0, 0)], vec![]),
            (5, vec![(0, 0)], 1, vec![], vec![(0, 0)]),
            (6, vec![(-5, 5)], 0, vec![(0, 5)], vec![(-5, 0)]),
            (7, vec![(-5, 5)], -5, vec![(-5, 5)], vec![(-5, -5)]),
            (8, vec![(-5, 5)], 5, vec![(5, 5)], vec![(-5, 5)]),
            (9, vec![(-5, -1), (1, 5)], 0, vec![(1, 5)], vec![(-5, -1)]),
            (
                10,
                vec![(-5, -1), (1, 5)],
                1,
                vec![(1, 5)],
                vec![(-5, -1), (1, 1)],
            ),
            (
                11,
                vec![(-5, -1), (1, 5)],
                -1,
                vec![(-1, -1), (1, 5)],
                vec![(-5, -1)],
            ),
            (
                12,
                vec![(min, -1), (1, 5)],
                min,
                vec![(min, -1), (1, 5)],
                vec![(min, min)],
            ),
            (
                13,
                vec![(min, -1), (1, 5)],
                max,
                vec![],
                vec![(min, -1), (1, 5)],
            ),
            (
                14,
                vec![(-5, -1), (1, max)],
                max,
                vec![(max, max)],
                vec![(-5, -1), (1, max)],
            ),
            (
                15,
                vec![(min, -1), (1, max)],
                0,
                vec![(1, max)],
                vec![(min, -1)],
            ),
            (
                16,
                vec![(-5, -3), (0, 1), (3, 5)],
                1,
                vec![(1, 1), (3, 5)],
                vec![(-5, -3), (0, 1)],
            ),
        ];

        for (id, a, v, expected_left, expected_right) in cases {
            test_binary_value_op(
                format!("test #{} of shrink_left", id),
                a.clone(),
                v,
                |x, v| x.shrink_left(v),
                expected_left,
            );
            test_binary_value_op(
                format!("test #{} of shrink_right", id),
                a,
                v,
                |x, v| x.shrink_right(v),
                expected_right,
            );
        }
    }

    #[test]
    fn test_subset() {
        // Note: the first number is the test id, so it should be easy to identify which test has failed.
        // The two first vectors are the operands and the four last are expected results (resp.
        // `a.is_subset(b)`, `b.is_subset(a)`, `a.is_proper_subset(b)` and `b.is_proper_subset(a)`.
        let cases = vec![
            // identity tests
            (1, vec![], vec![], true, true, false, false),
            (2, vec![], vec![(1, 2)], true, false, true, false),
            (3, vec![], vec![(1, 2), (7, 9)], true, false, true, false),
            (
                4,
                vec![(1, 2), (7, 9)],
                vec![(1, 2)],
                false,
                true,
                false,
                true,
            ),
            (
                5,
                vec![(1, 2), (7, 9)],
                vec![(1, 2), (7, 9)],
                true,
                true,
                false,
                false,
            ),
            // middle tests
            (
                6,
                vec![(2, 7)],
                vec![(1, 2), (7, 9)],
                false,
                false,
                false,
                false,
            ),
            (
                7,
                vec![(3, 7)],
                vec![(1, 2), (7, 9)],
                false,
                false,
                false,
                false,
            ),
            (
                8,
                vec![(4, 5)],
                vec![(1, 2), (7, 9)],
                false,
                false,
                false,
                false,
            ),
            (
                9,
                vec![(2, 8)],
                vec![(1, 2), (7, 9)],
                false,
                false,
                false,
                false,
            ),
            // mixed tests
            (
                10,
                vec![(-3, -1), (4, 5), (11, 12)],
                vec![(-2, -1), (7, 9)],
                false,
                false,
                false,
                false,
            ),
            (
                11,
                vec![(-3, 0), (3, 6), (10, 11)],
                vec![(-2, -1), (4, 4), (10, 10)],
                false,
                true,
                false,
                true,
            ),
            (
                12,
                vec![(-3, 0), (3, 6), (10, 11)],
                vec![(-3, 0), (3, 6), (10, 11)],
                true,
                true,
                false,
                false,
            ),
            // englobing tests
            (
                13,
                vec![(-1, 11)],
                vec![(1, 2), (7, 9)],
                false,
                true,
                false,
                true,
            ),
        ];

        for (id, a, b, expected, expected_sym, expected_proper, expected_proper_sym) in cases {
            test_binary_bool_op(
                format!("test #{} of subset", id),
                a.clone(),
                b.clone(),
                |x, y| x.is_subset(y),
                expected,
            );
            test_binary_bool_op(
                format!("test #{} of subset", id),
                b.clone(),
                a.clone(),
                |x, y| x.is_subset(y),
                expected_sym,
            );
            test_binary_bool_op(
                format!("test #{} of proper subset", id),
                a.clone(),
                b.clone(),
                |x, y| x.is_proper_subset(y),
                expected_proper,
            );
            test_binary_bool_op(
                format!("test #{} of proper subset", id),
                b.clone(),
                a.clone(),
                |x, y| x.is_proper_subset(y),
                expected_proper_sym,
            );
        }
    }

    #[test]
    fn test_arithmetics() {
        // Second and third args are the test values.
        // 4, 5, 6 and 7 are the results of `a + b`, `a - b`, `b - a` and `a * b`
        let cases = vec![
            (1, vec![], vec![], vec![], vec![], vec![], vec![]),
            (
                2,
                vec![],
                vec![(1, 1), (3, 5)],
                vec![],
                vec![],
                vec![],
                vec![],
            ),
            (
                3,
                vec![(1, 1), (3, 5)],
                vec![(0, 0)],
                vec![(1, 1), (3, 5)],
                vec![(1, 1), (3, 5)],
                vec![(-5, -3), (-1, -1)],
                vec![(0, 0)],
            ),
            (
                4,
                vec![(1, 1), (3, 5)],
                vec![(1, 1)],
                vec![(2, 2), (4, 6)],
                vec![(0, 0), (2, 4)],
                vec![(-4, -2), (0, 0)],
                vec![(1, 1), (3, 5)],
            ),
            (
                5,
                vec![(1, 1), (3, 5)],
                vec![(0, 1), (4, 5)],
                vec![(1, 10)],
                vec![(-4, 5)],
                vec![(-5, 4)],
                vec![(0, 5), (12, 25)],
            ),
        ];

        for (id, a, b, e_add, e_sub, e_sub_sym, e_mul) in cases {
            test_binary_op_sym(
                format!("test #{} of `a+b`", id),
                a.clone(),
                b.clone(),
                |x, y| x + y,
                e_add,
            );
            test_binary_op(
                format!("test #{} of `a-b`", id),
                a.clone(),
                b.clone(),
                |x, y| x - y,
                e_sub,
            );
            test_binary_op(
                format!("test #{} of `b-a`", id),
                b.clone(),
                a.clone(),
                |x, y| x - y,
                e_sub_sym,
            );
            test_binary_op_sym(
                format!("test #{} of `a*b`", id),
                a.clone(),
                b.clone(),
                |x, y| x * y,
                e_mul,
            );
        }
    }

    #[test]
    fn test_arithmetics_bound() {
        let i1_35 = vec![(1, 1), (3, 5)];
        // Second and third args are the test value.
        // 4, 5 and 6 are the results of `a + b`, `a - b` and `a * b`
        let cases = vec![
            (1, vec![], 0, vec![], vec![], vec![]),
            (2, vec![(0, 0)], 0, vec![(0, 0)], vec![(0, 0)], vec![(0, 0)]),
            (
                3,
                i1_35.clone(),
                0,
                i1_35.clone(),
                i1_35.clone(),
                vec![(0, 0)],
            ),
            (4, vec![], 1, vec![], vec![], vec![]),
            (
                5,
                vec![(0, 0)],
                1,
                vec![(1, 1)],
                vec![(-1, -1)],
                vec![(0, 0)],
            ),
            (
                6,
                i1_35.clone(),
                1,
                vec![(2, 2), (4, 6)],
                vec![(0, 0), (2, 4)],
                i1_35.clone(),
            ),
            (7, vec![], 3, vec![], vec![], vec![]),
            (
                8,
                vec![(0, 0)],
                3,
                vec![(3, 3)],
                vec![(-3, -3)],
                vec![(0, 0)],
            ),
            (
                9,
                i1_35.clone(),
                3,
                vec![(4, 4), (6, 8)],
                vec![(-2, -2), (0, 2)],
                vec![(3, 3), (9, 15)],
            ),
        ];

        for (id, a, b, e_add, e_sub, e_mul) in cases {
            test_binary_value_op(
                format!("test #{} of `a+b`", id),
                a.clone(),
                b.clone(),
                |x, y| x + y,
                e_add,
            );
            test_binary_value_op(
                format!("test #{} of `a-b`", id),
                a.clone(),
                b.clone(),
                |x, y| x - y,
                e_sub,
            );
            test_binary_value_op(
                format!("test #{} of `a*b`", id),
                a.clone(),
                b.clone(),
                |x, y| x * y,
                e_mul,
            );
        }
    }

    #[test]
    fn test_lattice() {
        use gcollections::ops::lattice::test::*;
        use trilean::SKleene::*;
        let whole = IntervalSet::<i32>::whole();
        let empty = IntervalSet::<i32>::empty();
        let a = vec![(0, 5), (10, 15)].to_interval_set();
        let b = vec![(5, 10)].to_interval_set();
        let c = vec![(6, 9)].to_interval_set();
        let d = vec![(4, 6), (8, 10)].to_interval_set();
        let e = vec![(5, 5), (10, 10)].to_interval_set();
        let f = vec![(6, 6), (8, 9)].to_interval_set();
        let g = vec![(0, 15)].to_interval_set();
        let h = vec![(4, 10)].to_interval_set();
        let tester = LatticeTester::new(
            0,
            /* data_a */
            vec![
                empty.clone(),
                empty.clone(),
                whole.clone(),
                a.clone(),
                b.clone(),
                c.clone(),
            ],
            /* data_b */
            vec![
                whole.clone(),
                a.clone(),
                a.clone(),
                b.clone(),
                c.clone(),
                d.clone(),
            ],
            /* a |= b*/ vec![True, True, False, Unknown, False, Unknown],
            /* a |_| b */
            vec![
                empty.clone(),
                empty.clone(),
                a.clone(),
                e.clone(),
                c.clone(),
                f.clone(),
            ],
            /* a |-| b */
            vec![
                whole.clone(),
                a.clone(),
                whole.clone(),
                g.clone(),
                b.clone(),
                h.clone(),
            ],
        );
        tester.test_all();
    }

    #[test]
    fn test_iterator() {
        let empty = IntervalSet::<i32>::empty();
        let mut a = [(0, 5), (10, 15)].to_interval_set();
        let mut iter = a.clone().into_iter();
        assert_eq!(
            iter.next(),
            Some(Interval::new(0, 5)),
            "test 1 of test_iterator"
        );
        assert_eq!(
            iter.next(),
            Some(Interval::new(10, 15)),
            "test 2 of test_iterator"
        );
        assert_eq!(iter.next(), None, "test 3 of test_iterator");
        for _x in a.clone() {}
        for _x in &a {}
        for _x in &mut a {}
        for _x in empty {
            assert!(
                false,
                "test 4 of test_iterator: empty interval must not yield an element."
            );
        }
    }

    #[test]
    fn test_ser_de_single_interval_set() {
        assert_tokens(
            &IntervalSet::from_interval(Interval::new(8, 12)),
            &[
                Token::Seq { len: Some(1) },
                Token::Tuple { len: 2 },
                Token::I32(8),
                Token::I32(12),
                Token::TupleEnd,
                Token::SeqEnd,
            ],
        );
    }

    #[test]
    fn test_ser_de_multiple_interval_set() {
        assert_tokens(
            &[(20, 21), (3, 5), (-10, -5)].to_interval_set(),
            &[
                Token::Seq { len: Some(3) },
                Token::Tuple { len: 2 },
                Token::I32(-10),
                Token::I32(-5),
                Token::TupleEnd,
                Token::Tuple { len: 2 },
                Token::I32(3),
                Token::I32(5),
                Token::TupleEnd,
                Token::Tuple { len: 2 },
                Token::I32(20),
                Token::I32(21),
                Token::TupleEnd,
                Token::SeqEnd,
            ],
        );
    }

    #[test]
    fn test_ser_de_empty_interval_set() {
        assert_tokens(&IntervalSet::<i32>::empty(), &[Token::None]);
    }
}
