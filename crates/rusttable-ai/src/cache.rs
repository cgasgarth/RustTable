use std::collections::{BTreeMap, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionKey {
    pub model: crate::ModelIdentity,
    pub qualification: [u8; 32],
    pub runtime_configuration: [u8; 32],
}

struct Entry<V> {
    value: V,
    bytes: u64,
}

pub struct SessionCache<K, V> {
    budget: u64,
    bytes: u64,
    entries: BTreeMap<K, Entry<V>>,
    lru: VecDeque<K>,
}

impl<K, V> SessionCache<K, V>
where
    K: Clone + Ord,
{
    #[must_use]
    pub fn new(budget: u64) -> Self {
        Self {
            budget,
            bytes: 0,
            entries: BTreeMap::new(),
            lru: VecDeque::new(),
        }
    }

    #[must_use]
    pub const fn budget(&self) -> u64 {
        self.budget
    }

    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        if !self.entries.contains_key(key) {
            return None;
        }
        self.touch(key);
        self.entries.get_mut(key).map(|entry| &mut entry.value)
    }

    pub fn insert(&mut self, key: K, value: V, bytes: u64) -> bool {
        if bytes > self.budget {
            return false;
        }
        if let Some(previous) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(previous.bytes);
            self.lru.retain(|candidate| candidate != &key);
        }
        while self.bytes.saturating_add(bytes) > self.budget {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(entry.bytes);
            }
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.lru.push_back(key.clone());
        self.entries.insert(key, Entry { value, bytes });
        true
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        self.lru.retain(|candidate| candidate != key);
        self.entries.remove(key).map(|entry| {
            self.bytes = self.bytes.saturating_sub(entry.bytes);
            entry.value
        })
    }

    fn touch(&mut self, key: &K) {
        self.lru.retain(|candidate| candidate != key);
        self.lru.push_back(key.clone());
    }
}
