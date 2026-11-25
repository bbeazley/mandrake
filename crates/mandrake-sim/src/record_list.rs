use std::fmt;
use std::ops::{Deref, DerefMut, RangeBounds};

use crate::projectile_groups::MAXIMUM_PROJECTILES;

pub trait RecordIndex {
    fn record_index(&self) -> i32;
}

impl RecordIndex for i32 {
    fn record_index(&self) -> i32 {
        *self
    }
}

impl<T: RecordIndex> RecordIndex for Box<T> {
    fn record_index(&self) -> i32 {
        (**self).record_index()
    }
}

const WORDS: usize = MAXIMUM_PROJECTILES / 64;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct UsedIndexes {
    counts: Vec<u32>,
    bits: [u64; WORDS],
    others: usize,
}

impl UsedIndexes {
    fn add(&mut self, index: i32) {
        match usize::try_from(index)
            .ok()
            .filter(|&i| i < MAXIMUM_PROJECTILES)
        {
            Some(i) => {
                if self.counts.is_empty() {
                    self.counts = vec![0; MAXIMUM_PROJECTILES];
                }
                self.counts[i] += 1;
                self.bits[i / 64] |= 1 << (i % 64);
            }
            None => self.others += 1,
        }
    }

    fn remove(&mut self, index: i32) {
        match usize::try_from(index)
            .ok()
            .filter(|&i| i < MAXIMUM_PROJECTILES)
        {
            Some(i) => {
                self.counts[i] -= 1;
                if self.counts[i] == 0 {
                    self.bits[i / 64] &= !(1 << (i % 64));
                }
            }
            None => self.others -= 1,
        }
    }

    fn clear(&mut self) {
        self.counts.iter_mut().for_each(|c| *c = 0);
        self.bits = [0; WORDS];
        self.others = 0;
    }
}

#[derive(Clone, Copy)]
pub struct UsedSet {
    bits: [u64; WORDS],
}

impl UsedSet {
    pub fn of<'a>(
        lists: impl IntoIterator<Item = &'a UsedIndexes>,
        extra: impl IntoIterator<Item = i32>,
    ) -> Option<Self> {
        let mut bits = [0u64; WORDS];
        for l in lists {
            if l.others != 0 {
                return None;
            }
            for (b, w) in bits.iter_mut().zip(l.bits) {
                *b |= w;
            }
        }
        for i in extra {
            let i = usize::try_from(i)
                .ok()
                .filter(|&i| i < MAXIMUM_PROJECTILES)?;
            bits[i / 64] |= 1 << (i % 64);
        }
        Some(Self { bits })
    }

    pub fn count(&self) -> usize {
        self.bits.iter().map(|w| w.count_ones() as usize).sum()
    }

    pub fn lowest_free(&self) -> i32 {
        let w = self
            .bits
            .iter()
            .position(|&w| w != u64::MAX)
            .unwrap_or(WORDS);
        (w * 64 + self.bits.get(w).map_or(0, |w| w.trailing_ones() as usize)) as i32
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RecordList<T> {
    items: Vec<T>,
    used: UsedIndexes,
}

impl<T> Default for RecordList<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            used: UsedIndexes::default(),
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for RecordList<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.items.fmt(f)
    }
}

impl<T> Deref for RecordList<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.items
    }
}

impl<T> DerefMut for RecordList<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.items
    }
}

impl<T: RecordIndex> From<Vec<T>> for RecordList<T> {
    fn from(items: Vec<T>) -> Self {
        let mut used = UsedIndexes::default();
        for t in &items {
            used.add(t.record_index());
        }
        Self { items, used }
    }
}

impl<T: RecordIndex> FromIterator<T> for RecordList<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::from(iter.into_iter().collect::<Vec<T>>())
    }
}

impl<T> IntoIterator for RecordList<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a RecordList<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut RecordList<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.iter_mut()
    }
}

impl<T: serde::Serialize> serde::Serialize for RecordList<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.items.serialize(s)
    }
}

impl<'de, T: RecordIndex + serde::Deserialize<'de>> serde::Deserialize<'de> for RecordList<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Vec::<T>::deserialize(d).map(Self::from)
    }
}

impl<T: RecordIndex> RecordList<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn used(&self) -> &UsedIndexes {
        &self.used
    }

    pub fn as_slice(&self) -> &[T] {
        &self.items
    }

    pub fn push(&mut self, t: T) {
        self.used.add(t.record_index());
        self.items.push(t);
    }

    pub fn pop(&mut self) -> Option<T> {
        let t = self.items.pop()?;
        self.used.remove(t.record_index());
        Some(t)
    }

    pub fn insert(&mut self, at: usize, t: T) {
        self.used.add(t.record_index());
        self.items.insert(at, t);
    }

    pub fn remove(&mut self, at: usize) -> T {
        let t = self.items.remove(at);
        self.used.remove(t.record_index());
        t
    }

    pub fn swap_remove(&mut self, at: usize) -> T {
        let t = self.items.swap_remove(at);
        self.used.remove(t.record_index());
        t
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.used.clear();
    }

    pub fn truncate(&mut self, len: usize) {
        for t in self.items.get(len..).unwrap_or(&[]) {
            self.used.remove(t.record_index());
        }
        self.items.truncate(len);
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&T) -> bool) {
        let used = &mut self.used;
        self.items.retain(|t| {
            let k = keep(t);
            if !k {
                used.remove(t.record_index());
            }
            k
        });
    }

    pub fn retain_mut(&mut self, mut keep: impl FnMut(&mut T) -> bool) {
        let used = &mut self.used;
        self.items.retain_mut(|t| {
            let k = keep(t);
            if !k {
                used.remove(t.record_index());
            }
            k
        });
    }

    pub fn drain(&mut self, range: impl RangeBounds<usize>) -> std::vec::IntoIter<T> {
        let out: Vec<T> = self.items.drain(range).collect();
        for t in &out {
            self.used.remove(t.record_index());
        }
        out.into_iter()
    }

    pub fn extend(&mut self, iter: impl IntoIterator<Item = T>) {
        for t in iter {
            self.push(t);
        }
    }

    pub fn append(&mut self, other: &mut Self) {
        let other = std::mem::take(other);
        self.extend(other.items);
    }

    pub fn split_off(&mut self, at: usize) -> Self {
        let tail = self.items.split_off(at);
        for t in &tail {
            self.used.remove(t.record_index());
        }
        Self::from(tail)
    }

    pub fn into_vec(self) -> Vec<T> {
        self.items
    }

    pub fn replace(&mut self, at: usize, t: T) -> T {
        self.used.add(t.record_index());
        let old = std::mem::replace(&mut self.items[at], t);
        self.used.remove(old.record_index());
        old
    }

    pub fn counted(&self) -> UsedIndexes {
        Self::from_counts(&self.items)
    }

    fn from_counts(items: &[T]) -> UsedIndexes {
        let mut used = UsedIndexes::default();
        for t in items {
            used.add(t.record_index());
        }
        used
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_follow_the_list() {
        let mut l: RecordList<i32> = RecordList::new();
        for i in [3, 0, 1, 3, 5000, -1] {
            l.push(i);
        }
        assert!(UsedSet::of([l.used()], []).is_none());
        l.retain(|&i| (0..1024).contains(&i));
        let s = UsedSet::of([l.used()], []).unwrap();
        assert_eq!((s.count(), s.lowest_free()), (3, 2));
        l.remove(0);
        let s = UsedSet::of([l.used()], [2]).unwrap();
        assert_eq!((s.count(), s.lowest_free()), (4, 4));
        l.pop();
        l.truncate(0);
        let s = UsedSet::of([l.used()], []).unwrap();
        assert_eq!((s.count(), s.lowest_free()), (0, 0));
        let full: RecordList<i32> = (0..1024).collect();
        let s = UsedSet::of([full.used()], []).unwrap();
        assert_eq!((s.count(), s.lowest_free()), (1024, 1024));
        assert!(full.used() == &full.counted());
    }
}
