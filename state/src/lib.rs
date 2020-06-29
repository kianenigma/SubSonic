use std::cell::RefCell;
use std::collections::hash_map::HashMap;
use std::fmt::Debug;
use std::sync::{Arc, RwLock};

// FIXME: some means of computing the state root.

/// Public interface of a state database. It could in principle use any backend.
pub trait GenericState<K, V, T> {
	/// Read the state entry at `key`.
	///
	/// - If the key does not exist, it will try and taint it, and return `Ok(Default)`.
	/// 	- This will first create a read lock, then a write lock.
	/// - If the key exists, and the taint is equal to `current`, then Ok(value) is returned.
	/// 	- This will only require read locks.
	/// - If the key exists, and the taint is **not** equal to `current`, then `Err(owner)` is
	///   returned.
	/// 	- This will only require read locks.
	fn read(&self, key: &K, current: T) -> Result<V, T>;

	/// Write to the state entry at `key`.
	///
	/// - If the key does not exist, it will try and taint it, then write `value` into it and return
	///   `Ok(())`.
	/// 	- This will first create a read lock, then a write lock.
	/// - If the key exists, and the taint is equal to `current`, then the `value` is written and
	///   Ok(()) is returned.
	/// 	- This will only require read locks.
	/// - If the key exists, and the taint is **not** equal to `current`, then `Err(owner)` is
	///   returned.
	/// 	- This will only require read locks.
	fn write(&self, key: &K, value: V, current: T) -> Result<(), T>;

	/// A combination of read and write, in place. Just a syntactic sugar, not really optimized.
	fn mutate(&self, key: &K, update: impl Fn(&mut V) -> (), current: T) -> Result<(), T>
	where
		K: KeyT,
		V: ValueT,
		T: TaintT,
	{
		self.read(key, current).and_then(|mut val| {
			(update)(&mut val);
			self.write(key, val, current)
		})
	}
}

type MapType<K, V, T> = HashMap<K, StateEntry<V, T>>;

pub trait KeyT: Clone + Debug + std::hash::Hash + Eq + PartialEq {}
impl<T: Clone + Debug + std::hash::Hash + Eq + PartialEq> KeyT for T {}

pub trait ValueT: Clone + Debug + Default {}
impl<T: Clone + Debug + Default> ValueT for T {}

pub trait TaintT: Clone + Copy + Debug + Eq + PartialEq {}
impl<T: Clone + Copy + Debug + Eq + PartialEq> TaintT for T {}

#[derive(Default, Debug, Clone)]
pub struct StateEntry<V, T> {
	data: RefCell<V>,
	taint: T,
}

// or just use atomic RefCell.
unsafe impl<V, T> Sync for StateEntry<V, T> {}

impl<V: ValueT, T: TaintT> StateEntry<V, T> {
	pub fn new(value: V, taint: T) -> Self {
		Self {
			data: value.into(),
			taint,
		}
	}

	pub fn new_taint(taint: T) -> Self {
		Self {
			taint,
			data: Default::default(),
		}
	}
}

/// A struct that implements `GenericState`.
///
/// This implements the taintable struct. Each access will try and taint that state key. Any further
/// access from other threads will not be allowed.
///
/// This is a highly concurrent implementation. Locking is scarce.
#[derive(Debug, Default)]
pub struct TaintState<K: KeyT, V: ValueT, T: TaintT> {
	backend: RwLock<MapType<K, V, T>>,
}

impl<K: KeyT, V: ValueT, T: TaintT> TaintState<K, V, T> {
	/// Create a new `TaintState`.
	pub fn new() -> Self {
		Self {
			backend: RwLock::new(MapType::default()),
		}
	}

	/// Consume self and return it wrapped in an `Arc`.
	pub fn as_arc(self) -> Arc<Self> {
		std::sync::Arc::new(self)
	}

	/// Create self with given capacity.
	pub fn with_capacity(capacity: usize) -> Self {
		Self {
			backend: RwLock::new(MapType::with_capacity(capacity)),
		}
	}

	/// Unsafe implementation of insert. This will not respect the tainting of the key.
	pub fn unsafe_insert(&self, at: &K, value: StateEntry<V, T>) {
		self.backend.write().unwrap().insert(at.clone(), value);
	}

	/// Unsafe implementation of read. This will not respect the tainting of the key.
	pub fn unsafe_read_value(&self, key: &K) -> Option<V> {
		self.unsafe_read(key).map(|e| e.data.clone().into_inner())
	}

	/// Unsafe implementation of read. This will not respect the tainting of the key.
	pub fn unsafe_read_taint(&self, key: &K) -> Option<T> {
		self.unsafe_read(key).map(|e| e.taint)
	}

	/// Unsafe implementation of read. This will not respect the tainting of the key.
	fn unsafe_read(&self, key: &K) -> Option<StateEntry<V, T>> {
		self.backend.read().unwrap().get(key).cloned()
	}
}

impl<K: KeyT, V: ValueT, T: TaintT> GenericState<K, V, T> for TaintState<K, V, T> {
	fn read(&self, key: &K, current: T) -> Result<V, T> {
		// first, see if it exists with a write key.
		let read_guard = self.backend.read().unwrap();
		if let Some(entry) = read_guard.get(key) {
			let owner = entry.taint;
			if owner == current {
				Ok(entry.data.borrow().clone())
			} else {
				Err(owner)
			}
		} else {
			drop(read_guard);
			// acquire a write guard to taint.
			let mut write_guard = self.backend.write().unwrap();
			if let Some(entry) = write_guard.get(key) {
				// This should very rarely happen. We just checked that this key does not exits.
				// Nonetheless, someone might have sneaked in while we were waiting for a write
				// lock.
				let owner = entry.taint;
				if owner == current {
					panic!("Current thread cannot be the owner.");
				} else {
					Err(owner)
				}
			} else {
				// we have the write lock and the entry does not exist. Taint and move on.
				let new_entry = <StateEntry<V, T>>::new_taint(current);
				write_guard.insert(key.clone(), new_entry);
				Ok(Default::default())
			}
		}
	}

	fn write(&self, key: &K, value: V, current: T) -> Result<(), T> {
		let read_guard = self.backend.read().unwrap();
		if let Some(entry) = read_guard.get(key) {
			let owner = entry.taint;
			if owner == current {
				*entry.data.borrow_mut() = value;
				Ok(())
			} else {
				Err(owner)
			}
		} else {
			drop(read_guard);
			let mut write_guard = self.backend.write().unwrap();
			if let Some(entry) = write_guard.get(key) {
				let owner = entry.taint;
				if owner == current {
					panic!("Current thread cannot be the owner.");
				} else {
					Err(owner)
				}
			} else {
				let new_entry = <StateEntry<V, T>>::new(value, current);
				write_guard.insert(key.clone(), new_entry);
				Ok(())
			}
		}
	}
}

#[cfg(test)]
mod test_state {
	use super::*;
	use std::sync::Arc;
	use std::thread;

	type Key = u32;
	type Value = u32;
	type ThreadId = u8;

	type TestState = TaintState<Key, Value, ThreadId>;

	#[test]
	fn basic_state_works() {
		let state = TaintState::new();
		state.unsafe_insert(&33, StateEntry::new("Foo", "Thread1"));
		assert_eq!(state.unsafe_read_value(&33).unwrap(), "Foo");
		assert_eq!(state.unsafe_read_taint(&33).unwrap(), "Thread1");
	}

	#[test]
	fn basic_read_write_ops() {
		let state = TestState::new();
		assert_eq!(state.read(&10, 1).unwrap(), 0);
		assert!(state.write(&10, 5, 1).is_ok());
		assert_eq!(state.read(&10, 1).unwrap(), 5);
	}

	#[test]
	fn reading_taints() {
		let state = TaintState::<u32, u32, u8>::new();
		assert!(state.read(&10u32, 1u8).is_ok());
		assert_eq!(state.unsafe_read_taint(&10).unwrap(), 1);
	}

	#[test]
	fn writing_taints() {
		let state = TaintState::new();
		assert!(state.write(&10u32, 5u32, 1u8).is_ok());
		assert_eq!(state.unsafe_read_taint(&10).unwrap(), 1);
	}

	#[test]
	fn cannot_read_from_tainted() {
		let state = TaintState::new();
		assert!(state.write(&10u32, 5u32, 1u8).is_ok());
		// thread 2 cannot read from 10 anymore.
		assert!(state.read(&10, 2).is_err());
	}

	#[test]
	fn can_share_state_between_threads() {
		let state = TestState::new().as_arc();

		let h1 = {
			let state = Arc::clone(&state);
			thread::spawn(move || state.write(&10, 10, 1))
		};

		let h2 = {
			let state = Arc::clone(&state);
			thread::spawn(move || state.write(&11, 11, 2))
		};

		assert!(h1.join().is_ok());
		assert!(h2.join().is_ok());

		assert_eq!(state.unsafe_read_value(&10).unwrap(), 10);
		assert_eq!(state.unsafe_read_value(&11).unwrap(), 11);
	}

	#[test]
	fn mutate_works() {
		let state = TestState::new().as_arc();

		assert!(state
			.mutate(
				&10,
				|old| {
					assert_eq!(*old, Value::default());
					*old = 5;
				},
				1
			)
			.is_ok());

		assert!(state
			.mutate(
				&10,
				|old| {
					assert_eq!(*old, 5);
					*old = 6;
				},
				1
			)
			.is_ok());

		state.unsafe_insert(&11, StateEntry::new(11, 2));

		assert!(state
			.mutate(
				&11,
				|_| {
					// closure will never be executed.
					assert!(false);
				},
				1
			)
			.is_err());
	}

	#[test]
	fn only_one_thread_can_taint_read() {
		let state = TestState::new().as_arc();
		let num_threads = 12;

		let handles: Vec<std::thread::JoinHandle<Result<Value, ThreadId>>> = (1..=num_threads)
			.map(|id| {
				let state = Arc::clone(&state);
				thread::spawn(move || state.read(&999, id))
			})
			.collect();

		let results: Vec<Result<Value, ThreadId>> =
			handles.into_iter().map(|h| h.join().unwrap()).collect();
		assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
		assert_eq!(
			results.iter().filter(|r| r.is_err()).count(),
			(num_threads - 1) as usize
		);
	}
}
