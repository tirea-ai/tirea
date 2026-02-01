//! Field proxy types for Accessor pattern.
//!
//! These types provide field-level access with read/write capabilities
//! and operator support for numeric types.

use crate::{CarveResult, CarveError, Op, Path};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;
use std::marker::PhantomData;
use std::ops::{AddAssign, SubAssign, MulAssign, DivAssign};

/// Internal helper to get value at path.
fn get_at_path<'a>(doc: &'a Value, path: &Path) -> Option<&'a Value> {
    crate::get_at_path(doc, path)
}

/// Internal helper to deserialize value.
fn deserialize_at<T: DeserializeOwned>(doc: &Value, path: &Path) -> CarveResult<T> {
    match get_at_path(doc, path) {
        Some(v) => serde_json::from_value(v.clone()).map_err(CarveError::from),
        None => Err(CarveError::PathNotFound { path: path.clone() }),
    }
}

// ============================================================================
// Scalar Field
// ============================================================================

/// A field proxy for scalar types (i32, i64, f64, bool, String, etc.).
pub struct ScalarField<'a, T> {
    doc: &'a Value,
    path: Path,
    ops: &'a RefCell<Vec<Op>>,
    _phantom: PhantomData<T>,
}

impl<'a, T> ScalarField<'a, T> {
    /// Create a new scalar field proxy.
    pub fn new(doc: &'a Value, path: Path, ops: &'a RefCell<Vec<Op>>) -> Self {
        Self {
            doc,
            path,
            ops,
            _phantom: PhantomData,
        }
    }
}

impl<'a, T: DeserializeOwned> ScalarField<'a, T> {
    /// Get the field value.
    pub fn get(&self) -> CarveResult<T> {
        deserialize_at(self.doc, &self.path)
    }

    /// Get the field value or a default if not present.
    pub fn get_or(&self, default: T) -> T {
        self.get().unwrap_or(default)
    }

    /// Check if the field exists.
    pub fn exists(&self) -> bool {
        get_at_path(self.doc, &self.path).is_some()
    }
}

impl<'a, T: Serialize> ScalarField<'a, T> {
    /// Set the field value.
    pub fn set(&self, value: T) {
        let json_value = serde_json::to_value(&value).unwrap_or(Value::Null);
        self.ops.borrow_mut().push(Op::set(self.path.clone(), json_value));
    }

    /// Delete the field.
    pub fn delete(&self) {
        self.ops.borrow_mut().push(Op::delete(self.path.clone()));
    }
}

// Numeric operators for i32
impl<'a> AddAssign<i32> for ScalarField<'a, i32> {
    fn add_assign(&mut self, rhs: i32) {
        self.ops.borrow_mut().push(Op::increment(self.path.clone(), rhs as i64));
    }
}

impl<'a> SubAssign<i32> for ScalarField<'a, i32> {
    fn sub_assign(&mut self, rhs: i32) {
        self.ops.borrow_mut().push(Op::decrement(self.path.clone(), rhs as i64));
    }
}

// Numeric operators for i64
impl<'a> AddAssign<i64> for ScalarField<'a, i64> {
    fn add_assign(&mut self, rhs: i64) {
        self.ops.borrow_mut().push(Op::increment(self.path.clone(), rhs));
    }
}

impl<'a> SubAssign<i64> for ScalarField<'a, i64> {
    fn sub_assign(&mut self, rhs: i64) {
        self.ops.borrow_mut().push(Op::decrement(self.path.clone(), rhs));
    }
}

// Numeric operators for f64
impl<'a> AddAssign<f64> for ScalarField<'a, f64> {
    fn add_assign(&mut self, rhs: f64) {
        self.ops.borrow_mut().push(Op::increment(self.path.clone(), rhs));
    }
}

impl<'a> SubAssign<f64> for ScalarField<'a, f64> {
    fn sub_assign(&mut self, rhs: f64) {
        self.ops.borrow_mut().push(Op::decrement(self.path.clone(), rhs));
    }
}

impl<'a> MulAssign<f64> for ScalarField<'a, f64> {
    fn mul_assign(&mut self, rhs: f64) {
        // Multiply requires read-then-write
        if let Ok(current) = self.get() {
            self.set(current * rhs);
        }
    }
}

impl<'a> DivAssign<f64> for ScalarField<'a, f64> {
    fn div_assign(&mut self, rhs: f64) {
        // Divide requires read-then-write
        if let Ok(current) = self.get() {
            self.set(current / rhs);
        }
    }
}

// ============================================================================
// Option Field
// ============================================================================

/// A field proxy for Option<T> types.
pub struct OptionField<'a, T> {
    doc: &'a Value,
    path: Path,
    ops: &'a RefCell<Vec<Op>>,
    _phantom: PhantomData<T>,
}

impl<'a, T> OptionField<'a, T> {
    /// Create a new option field proxy.
    pub fn new(doc: &'a Value, path: Path, ops: &'a RefCell<Vec<Op>>) -> Self {
        Self {
            doc,
            path,
            ops,
            _phantom: PhantomData,
        }
    }
}

impl<'a, T: DeserializeOwned> OptionField<'a, T> {
    /// Get the field value.
    pub fn get(&self) -> CarveResult<Option<T>> {
        match get_at_path(self.doc, &self.path) {
            Some(Value::Null) | None => Ok(None),
            Some(v) => {
                let val: T = serde_json::from_value(v.clone())?;
                Ok(Some(val))
            }
        }
    }

    /// Check if the field has a value (not null and exists).
    pub fn is_some(&self) -> bool {
        matches!(get_at_path(self.doc, &self.path), Some(v) if !v.is_null())
    }

    /// Check if the field is None (null or not exists).
    pub fn is_none(&self) -> bool {
        !self.is_some()
    }
}

impl<'a, T: Serialize> OptionField<'a, T> {
    /// Set the field value to Some(value).
    pub fn set(&self, value: T) {
        let json_value = serde_json::to_value(&value).unwrap_or(Value::Null);
        self.ops.borrow_mut().push(Op::set(self.path.clone(), json_value));
    }

    /// Set the field value to None (null).
    pub fn set_none(&self) {
        self.ops.borrow_mut().push(Op::set(self.path.clone(), Value::Null));
    }

    /// Delete the field entirely.
    pub fn delete(&self) {
        self.ops.borrow_mut().push(Op::delete(self.path.clone()));
    }
}

// ============================================================================
// Vec Field
// ============================================================================

/// A field proxy for Vec<T> types.
pub struct VecField<'a, T> {
    doc: &'a Value,
    path: Path,
    ops: &'a RefCell<Vec<Op>>,
    _phantom: PhantomData<T>,
}

impl<'a, T> VecField<'a, T> {
    /// Create a new vec field proxy.
    pub fn new(doc: &'a Value, path: Path, ops: &'a RefCell<Vec<Op>>) -> Self {
        Self {
            doc,
            path,
            ops,
            _phantom: PhantomData,
        }
    }
}

impl<'a, T: DeserializeOwned> VecField<'a, T> {
    /// Get all items in the vec.
    pub fn get(&self) -> CarveResult<Vec<T>> {
        deserialize_at(self.doc, &self.path)
    }

    /// Get the item at the specified index.
    pub fn get_at(&self, index: usize) -> CarveResult<T> {
        let path = self.path.clone().index(index);
        deserialize_at(self.doc, &path)
    }

    /// Get the length of the vec.
    pub fn len(&self) -> CarveResult<usize> {
        match get_at_path(self.doc, &self.path) {
            Some(Value::Array(arr)) => Ok(arr.len()),
            Some(_) => Err(CarveError::TypeMismatch {
                path: self.path.clone(),
                expected: "array",
                found: "other",
            }),
            None => Ok(0),
        }
    }

    /// Check if the vec is empty.
    pub fn is_empty(&self) -> CarveResult<bool> {
        Ok(self.len()? == 0)
    }

    /// Check if the vec exists.
    pub fn exists(&self) -> bool {
        matches!(get_at_path(self.doc, &self.path), Some(Value::Array(_)))
    }
}

impl<'a, T: Serialize> VecField<'a, T> {
    /// Set the entire vec.
    pub fn set(&self, vec: Vec<T>) {
        let json_value = serde_json::to_value(&vec).unwrap_or(Value::Array(vec![]));
        self.ops.borrow_mut().push(Op::set(self.path.clone(), json_value));
    }

    /// Push an item to the end of the vec.
    pub fn push(&self, item: T) {
        let json_value = serde_json::to_value(&item).unwrap_or(Value::Null);
        self.ops.borrow_mut().push(Op::append(self.path.clone(), json_value));
    }

    /// Set the item at the specified index.
    pub fn set_at(&self, index: usize, item: T) {
        let path = self.path.clone().index(index);
        let json_value = serde_json::to_value(&item).unwrap_or(Value::Null);
        self.ops.borrow_mut().push(Op::set(path, json_value));
    }

    /// Insert an item at the specified index.
    pub fn insert(&self, index: usize, item: T) {
        let json_value = serde_json::to_value(&item).unwrap_or(Value::Null);
        self.ops.borrow_mut().push(Op::insert(self.path.clone(), index, json_value));
    }

    /// Remove the first occurrence of an item.
    pub fn remove(&self, item: &T) {
        let json_value = serde_json::to_value(item).unwrap_or(Value::Null);
        self.ops.borrow_mut().push(Op::remove(self.path.clone(), json_value));
    }

    /// Delete the item at the specified index.
    pub fn delete_at(&self, index: usize) {
        let path = self.path.clone().index(index);
        self.ops.borrow_mut().push(Op::delete(path));
    }

    /// Clear the entire vec (set to empty array).
    pub fn clear(&self) {
        self.set(vec![]);
    }

    /// Delete the entire vec field.
    pub fn delete(&self) {
        self.ops.borrow_mut().push(Op::delete(self.path.clone()));
    }
}

// ============================================================================
// Map Field (HashMap / BTreeMap)
// ============================================================================

/// A field proxy for Map<K, V> types (HashMap or BTreeMap with String keys).
pub struct MapField<'a, K, V> {
    doc: &'a Value,
    path: Path,
    ops: &'a RefCell<Vec<Op>>,
    _phantom: PhantomData<(K, V)>,
}

impl<'a, K, V> MapField<'a, K, V> {
    /// Create a new map field proxy.
    pub fn new(doc: &'a Value, path: Path, ops: &'a RefCell<Vec<Op>>) -> Self {
        Self {
            doc,
            path,
            ops,
            _phantom: PhantomData,
        }
    }
}

impl<'a, V: DeserializeOwned> MapField<'a, String, V> {
    /// Get all entries as a HashMap.
    pub fn get(&self) -> CarveResult<HashMap<String, V>> {
        deserialize_at(self.doc, &self.path)
    }

    /// Get all entries as a BTreeMap.
    pub fn get_sorted(&self) -> CarveResult<BTreeMap<String, V>> {
        deserialize_at(self.doc, &self.path)
    }

    /// Get the value for a specific key.
    pub fn get_key(&self, key: &str) -> CarveResult<V> {
        let path = self.path.clone().key(key);
        deserialize_at(self.doc, &path)
    }

    /// Get the value for a specific key, or None if not present.
    pub fn get_key_opt(&self, key: &str) -> Option<V> {
        self.get_key(key).ok()
    }

    /// Check if a key exists.
    pub fn contains_key(&self, key: &str) -> bool {
        let path = self.path.clone().key(key);
        get_at_path(self.doc, &path).is_some()
    }

    /// Get all keys.
    pub fn keys(&self) -> CarveResult<Vec<String>> {
        match get_at_path(self.doc, &self.path) {
            Some(Value::Object(obj)) => Ok(obj.keys().cloned().collect()),
            Some(_) => Err(CarveError::TypeMismatch {
                path: self.path.clone(),
                expected: "object",
                found: "other",
            }),
            None => Ok(vec![]),
        }
    }

    /// Get the number of entries.
    pub fn len(&self) -> CarveResult<usize> {
        match get_at_path(self.doc, &self.path) {
            Some(Value::Object(obj)) => Ok(obj.len()),
            Some(_) => Err(CarveError::TypeMismatch {
                path: self.path.clone(),
                expected: "object",
                found: "other",
            }),
            None => Ok(0),
        }
    }

    /// Check if the map is empty.
    pub fn is_empty(&self) -> CarveResult<bool> {
        Ok(self.len()? == 0)
    }

    /// Check if the map field exists.
    pub fn exists(&self) -> bool {
        matches!(get_at_path(self.doc, &self.path), Some(Value::Object(_)))
    }
}

impl<'a, V: Serialize> MapField<'a, String, V> {
    /// Set the entire map (HashMap).
    pub fn set(&self, map: HashMap<String, V>) {
        let json_value = serde_json::to_value(&map).unwrap_or(Value::Object(Default::default()));
        self.ops.borrow_mut().push(Op::set(self.path.clone(), json_value));
    }

    /// Set the entire map (BTreeMap).
    pub fn set_sorted(&self, map: BTreeMap<String, V>) {
        let json_value = serde_json::to_value(&map).unwrap_or(Value::Object(Default::default()));
        self.ops.borrow_mut().push(Op::set(self.path.clone(), json_value));
    }

    /// Insert or update a key-value pair.
    pub fn insert(&self, key: impl Into<String>, value: V) {
        let path = self.path.clone().key(key.into());
        let json_value = serde_json::to_value(&value).unwrap_or(Value::Null);
        self.ops.borrow_mut().push(Op::set(path, json_value));
    }

    /// Remove a key.
    pub fn remove(&self, key: &str) {
        let path = self.path.clone().key(key);
        self.ops.borrow_mut().push(Op::delete(path));
    }

    /// Merge another map into this one.
    pub fn merge(&self, other: HashMap<String, V>) {
        let json_value = serde_json::to_value(&other).unwrap_or(Value::Object(Default::default()));
        self.ops.borrow_mut().push(Op::merge_object(self.path.clone(), json_value));
    }

    /// Clear the map (set to empty object).
    pub fn clear(&self) {
        self.set(HashMap::new());
    }

    /// Delete the entire map field.
    pub fn delete(&self) {
        self.ops.borrow_mut().push(Op::delete(self.path.clone()));
    }
}

// ============================================================================
// HashSet Field (stored as array in JSON)
// ============================================================================

/// A field proxy for HashSet<T> types (stored as array in JSON).
pub struct SetField<'a, T> {
    doc: &'a Value,
    path: Path,
    ops: &'a RefCell<Vec<Op>>,
    _phantom: PhantomData<T>,
}

impl<'a, T> SetField<'a, T> {
    /// Create a new set field proxy.
    pub fn new(doc: &'a Value, path: Path, ops: &'a RefCell<Vec<Op>>) -> Self {
        Self {
            doc,
            path,
            ops,
            _phantom: PhantomData,
        }
    }
}

impl<'a, T: DeserializeOwned + Eq + Hash> SetField<'a, T> {
    /// Get all items as a HashSet.
    pub fn get(&self) -> CarveResult<std::collections::HashSet<T>> {
        let vec: Vec<T> = deserialize_at(self.doc, &self.path)?;
        Ok(vec.into_iter().collect())
    }

    /// Check if the set contains an item.
    pub fn contains(&self, item: &T) -> CarveResult<bool>
    where
        T: PartialEq,
    {
        let vec: Vec<T> = self.get()?.into_iter().collect();
        Ok(vec.contains(item))
    }

    /// Get the number of items.
    pub fn len(&self) -> CarveResult<usize> {
        match get_at_path(self.doc, &self.path) {
            Some(Value::Array(arr)) => Ok(arr.len()),
            Some(_) => Err(CarveError::TypeMismatch {
                path: self.path.clone(),
                expected: "array",
                found: "other",
            }),
            None => Ok(0),
        }
    }

    /// Check if the set is empty.
    pub fn is_empty(&self) -> CarveResult<bool> {
        Ok(self.len()? == 0)
    }
}

impl<'a, T: Serialize + Eq + Hash> SetField<'a, T> {
    /// Set the entire set.
    pub fn set(&self, set: std::collections::HashSet<T>) {
        let vec: Vec<_> = set.into_iter().collect();
        let json_value = serde_json::to_value(&vec).unwrap_or(Value::Array(vec![]));
        self.ops.borrow_mut().push(Op::set(self.path.clone(), json_value));
    }

    /// Insert an item (append to array).
    pub fn insert(&self, item: T) {
        let json_value = serde_json::to_value(&item).unwrap_or(Value::Null);
        self.ops.borrow_mut().push(Op::append(self.path.clone(), json_value));
    }

    /// Remove an item.
    pub fn remove(&self, item: &T) {
        let json_value = serde_json::to_value(item).unwrap_or(Value::Null);
        self.ops.borrow_mut().push(Op::remove(self.path.clone(), json_value));
    }

    /// Clear the set.
    pub fn clear(&self) {
        self.set(std::collections::HashSet::new());
    }

    /// Delete the entire set field.
    pub fn delete(&self) {
        self.ops.borrow_mut().push(Op::delete(self.path.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_scalar_field() {
        let doc = json!({"counter": 42});
        let ops = RefCell::new(Vec::new());
        let field: ScalarField<'_, i32> = ScalarField::new(&doc, Path::root().key("counter"), &ops);

        assert_eq!(field.get().unwrap(), 42);
        assert!(field.exists());

        field.set(100);
        assert_eq!(ops.borrow().len(), 1);
    }

    #[test]
    fn test_vec_field() {
        let doc = json!({"items": [1, 2, 3]});
        let ops = RefCell::new(Vec::new());
        let field: VecField<'_, i32> = VecField::new(&doc, Path::root().key("items"), &ops);

        assert_eq!(field.get().unwrap(), vec![1, 2, 3]);
        assert_eq!(field.len().unwrap(), 3);
        assert_eq!(field.get_at(1).unwrap(), 2);

        field.push(4);
        field.insert(0, 0);
        assert_eq!(ops.borrow().len(), 2);
    }

    #[test]
    fn test_map_field() {
        let doc = json!({"metadata": {"key1": "value1", "key2": "value2"}});
        let ops = RefCell::new(Vec::new());
        let field: MapField<'_, String, String> = MapField::new(&doc, Path::root().key("metadata"), &ops);

        assert_eq!(field.get_key("key1").unwrap(), "value1");
        assert!(field.contains_key("key2"));
        assert_eq!(field.len().unwrap(), 2);

        field.insert("key3", "value3".to_string());
        field.remove("key1");
        assert_eq!(ops.borrow().len(), 2);
    }

    #[test]
    fn test_numeric_operators() {
        let doc = json!({"counter": 10});
        let ops = RefCell::new(Vec::new());
        let mut field: ScalarField<'_, i32> = ScalarField::new(&doc, Path::root().key("counter"), &ops);

        field += 5;
        field -= 3;

        let ops_vec = ops.borrow();
        assert_eq!(ops_vec.len(), 2);
    }
}
