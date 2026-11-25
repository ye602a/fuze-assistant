use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    #[default]
    Three,
    Four,
}

impl Layout {
    pub const fn count(self) -> usize {
        match self {
            Self::Three => 3,
            Self::Four => 4,
        }
    }

    pub fn empty_slots<T: Default>(self) -> CardSlots<T> {
        CardSlots::from_fn(self, |_| T::default())
    }
}

/// Ordered positions, including unreadable cards. The array representation keeps
/// existing three-card worker replies and saved observations readable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CardSlots<T> {
    Three([T; 3]),
    Four([T; 4]),
}

impl<T> CardSlots<T> {
    pub fn layout(&self) -> Layout {
        match self {
            Self::Three(_) => Layout::Three,
            Self::Four(_) => Layout::Four,
        }
    }

    pub fn from_fn(layout: Layout, f: impl FnMut(usize) -> T) -> Self {
        match layout {
            Layout::Three => Self::Three(std::array::from_fn(f)),
            Layout::Four => Self::Four(std::array::from_fn(f)),
        }
    }

    pub fn map<U>(self, f: impl FnMut(T) -> U) -> CardSlots<U> {
        match self {
            Self::Three(values) => CardSlots::Three(values.map(f)),
            Self::Four(values) => CardSlots::Four(values.map(f)),
        }
    }

    pub fn each_ref(&self) -> CardSlots<&T> {
        match self {
            Self::Three(values) => CardSlots::Three(values.each_ref()),
            Self::Four(values) => CardSlots::Four(values.each_ref()),
        }
    }
}

impl<T: Default> Default for CardSlots<T> {
    fn default() -> Self {
        Layout::Three.empty_slots()
    }
}

impl<T> From<[T; 3]> for CardSlots<T> {
    fn from(values: [T; 3]) -> Self {
        Self::Three(values)
    }
}

impl<T> From<[T; 4]> for CardSlots<T> {
    fn from(values: [T; 4]) -> Self {
        Self::Four(values)
    }
}

impl<T> Deref for CardSlots<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        match self {
            Self::Three(values) => values,
            Self::Four(values) => values,
        }
    }
}

impl<T> DerefMut for CardSlots<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        match self {
            Self::Three(values) => values,
            Self::Four(values) => values,
        }
    }
}

impl<'a, T> IntoIterator for &'a CardSlots<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_slots_preserve_the_old_array_and_four_card_holes() {
        for json in ["[1,null,3]", "[1,null,3,4]"] {
            let slots: CardSlots<Option<u32>> = serde_json::from_str(json).unwrap();
            assert_eq!(slots[1], None);
            assert_eq!(serde_json::to_string(&slots).unwrap(), json);
        }
        assert!(serde_json::from_str::<CardSlots<u32>>("[1,2]").is_err());
        assert!(serde_json::from_str::<CardSlots<u32>>("[1,2,3,4,5]").is_err());
    }
}
