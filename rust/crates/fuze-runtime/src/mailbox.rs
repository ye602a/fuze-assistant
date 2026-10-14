//! One pending value, not an unbounded frame queue. Producer and consumer serialize access.
pub struct Latest<T> {
    generation: u64,
    last_sequence: Option<u64>,
    value: Option<(u64, T)>,
}
impl<T> Latest<T> {
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            last_sequence: None,
            value: None,
        }
    }
    pub fn reset(&mut self, generation: u64) {
        self.generation = generation;
        self.last_sequence = None;
        self.value = None;
    }
    pub fn offer(&mut self, generation: u64, sequence: u64, value: T) -> bool {
        if generation != self.generation || self.last_sequence.is_some_and(|old| sequence <= old) {
            return false;
        }
        self.last_sequence = Some(sequence);
        self.value = Some((sequence, value));
        true
    }
    pub fn take(&mut self) -> Option<(u64, T)> {
        self.value.take()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn newest_wins_and_reset_rejects_late_old_producers() {
        let mut q = Latest::new(1);
        assert!(q.offer(1, 1, "first"));
        assert!(q.offer(1, 3, "latest"));
        assert!(!q.offer(1, 2, "stale"));
        assert_eq!(q.take(), Some((3, "latest")));
        assert!(!q.offer(1, 3, "duplicate"));
        q.reset(2);
        assert!(!q.offer(1, 99, "previous game"));
        assert!(q.offer(2, 1, "new game"));
        q.reset(3);
        assert_eq!(q.take(), None);
    }
}
