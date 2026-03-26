//! Task registry for type-based serialization and deserialization.
//!
//! [`SerializableTask`] is an opt-in trait for concrete task structs that
//! exposes construction parameters as JSON. [`TaskRegistry`] maps type-name
//! strings to factory closures that reconstruct tasks from those parameters.
//!
//! # Pattern
//!
//! Follows the same registry pattern as `unshape-serde`'s `NodeRegistry`.
//!
//! # Nesting
//!
//! Wrapper / decorator tasks that hold an inner [`SharedDynTask`] can serialize
//! their inner task by capturing a `Arc<TaskRegistry>` in their factory and
//! calling `registry.deserialize(type_name, params)` to reconstruct the inner
//! task. This makes recursive serialization of composed task graphs
//! straightforward.

use std::collections::HashMap;

use serde_json::Value as JsonValue;

use crate::dyn_task::SharedDynTask;

// ─── errors ──────────────────────────────────────────────────────────────────

/// Error from the task registry.
#[derive(Debug)]
pub enum RegistryError {
    /// No factory registered for this type name.
    UnknownTaskType(String),
    /// The factory failed (e.g. bad JSON shape).
    DeserializationFailed(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::UnknownTaskType(name) => {
                write!(f, "no factory registered for task type {name:?}")
            }
            RegistryError::DeserializationFailed(msg) => {
                write!(f, "task deserialization failed: {msg}")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

// ─── SerializableTask ─────────────────────────────────────────────────────────

/// Trait for task structs that can be serialized.
///
/// Implement this on your **concrete task structs** (not on `ErasedTask`) to
/// enable round-trip serialization/deserialization via [`TaskRegistry`].
///
/// The `serializable_type_name` must match the key passed to
/// [`TaskRegistry::register_factory`] / [`TaskRegistry::register_with_name`].
///
/// # Example
///
/// ```
/// use nanites_core::registry::SerializableTask;
/// use serde_json::json;
///
/// #[derive(Debug, Clone)]
/// struct TranslateText { target_language: String }
///
/// impl SerializableTask for TranslateText {
///     fn serializable_type_name(&self) -> &'static str { "myapp::TranslateText" }
///     fn params(&self) -> serde_json::Value {
///         json!({ "target_language": self.target_language })
///     }
/// }
/// ```
pub trait SerializableTask {
    /// Stable, human-readable type name used as the registry key.
    ///
    /// Unlike `std::any::type_name::<T>()`, this is under your control and
    /// will not change across refactors.
    fn serializable_type_name(&self) -> &'static str;

    /// Snapshot of all parameters needed to reconstruct this task.
    ///
    /// The returned JSON should contain every field that affects behaviour —
    /// think of it as the constructor arguments, not the runtime state.
    fn params(&self) -> JsonValue;
}

// ─── TaskSnapshot ─────────────────────────────────────────────────────────────

/// A serialized snapshot of a single task (type name + params).
///
/// This is the portable, storable form. Pass it to
/// [`TaskRegistry::deserialize_snapshot`] to reconstruct the live task.
#[derive(Debug, Clone)]
pub struct TaskSnapshot {
    /// Stable type name, used as the registry lookup key.
    pub type_name: String,
    /// Constructor parameters as a JSON value.
    pub params: JsonValue,
}

impl TaskSnapshot {
    /// Build a snapshot from a [`SerializableTask`].
    pub fn from_task(task: &dyn SerializableTask) -> Self {
        Self {
            type_name: task.serializable_type_name().to_string(),
            params: task.params(),
        }
    }

    /// Serialize this snapshot to a JSON string.
    pub fn to_json(&self) -> serde_json::Result<String> {
        let obj = serde_json::json!({
            "type_name": self.type_name,
            "params": self.params,
        });
        serde_json::to_string(&obj)
    }

    /// Deserialize a snapshot from a JSON string.
    ///
    /// Returns an error if the JSON is malformed or is missing `type_name`.
    pub fn from_json(s: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let v: serde_json::Value = serde_json::from_str(s)?;
        let type_name = v["type_name"]
            .as_str()
            .ok_or("missing or non-string 'type_name' field")?
            .to_string();
        Ok(Self {
            type_name,
            params: v["params"].clone(),
        })
    }
}

// ─── TaskRegistry ─────────────────────────────────────────────────────────────

/// Type alias for task factory closures.
type TaskFactory = Box<dyn Fn(JsonValue) -> Result<SharedDynTask, RegistryError> + Send + Sync>;

/// Registry that maps type-name strings to task factories.
///
/// Used to deserialize tasks from JSON snapshots. Build one at startup,
/// register all task types, then share it wherever tasks need to be
/// reconstructed from storage or wire format.
pub struct TaskRegistry {
    factories: HashMap<String, TaskFactory>,
}

impl TaskRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    /// Register a task type using a custom factory closure.
    ///
    /// The factory receives the JSON params and must return a [`SharedDynTask`].
    /// This is the primary registration method; use it for any task that needs
    /// non-trivial construction logic or wraps an inner task.
    pub fn register_factory<F>(&mut self, type_name: &str, factory: F)
    where
        F: Fn(JsonValue) -> Result<SharedDynTask, RegistryError> + Send + Sync + 'static,
    {
        self.factories
            .insert(type_name.to_string(), Box::new(factory));
    }

    /// Register a task type by explicit name using `serde_json` for
    /// deserialization.
    ///
    /// `T` must implement [`serde::de::DeserializeOwned`]. The params JSON is
    /// deserialized directly into `T` via `serde_json::from_value`, then
    /// wrapped in [`crate::dyn_task::erase`].
    ///
    /// This is the convenience path for simple tasks that derive
    /// `serde::Deserialize`.
    pub fn register_with_name<T>(&mut self, type_name: &str)
    where
        T: crate::task::Task + serde::de::DeserializeOwned + std::fmt::Debug + Clone + 'static,
        T::Error: std::error::Error + Send + Sync + 'static,
    {
        self.factories.insert(
            type_name.to_string(),
            Box::new(|params| {
                let task: T = serde_json::from_value(params)
                    .map_err(|e| RegistryError::DeserializationFailed(e.to_string()))?;
                Ok(crate::dyn_task::erase(task))
            }),
        );
    }

    /// Deserialize a task by type name and params JSON.
    ///
    /// Returns [`RegistryError::UnknownTaskType`] if the type name has not
    /// been registered.
    pub fn deserialize(
        &self,
        type_name: &str,
        params: JsonValue,
    ) -> Result<SharedDynTask, RegistryError> {
        let factory = self
            .factories
            .get(type_name)
            .ok_or_else(|| RegistryError::UnknownTaskType(type_name.to_string()))?;
        factory(params)
    }

    /// Deserialize a task from a [`TaskSnapshot`].
    pub fn deserialize_snapshot(
        &self,
        snapshot: &TaskSnapshot,
    ) -> Result<SharedDynTask, RegistryError> {
        self.deserialize(&snapshot.type_name, snapshot.params.clone())
    }

    /// Returns `true` if a factory is registered for `type_name`.
    pub fn contains(&self, type_name: &str) -> bool {
        self.factories.contains_key(type_name)
    }

    /// Iterator over all registered type names.
    pub fn registered_types(&self) -> impl Iterator<Item = &str> {
        self.factories.keys().map(String::as_str)
    }

    /// Number of registered type names.
    pub fn len(&self) -> usize {
        self.factories.len()
    }

    /// Returns `true` if no types are registered.
    pub fn is_empty(&self) -> bool {
        self.factories.is_empty()
    }
}

impl Default for TaskRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dyn_task::erase;
    use crate::{Ctx, Task};

    #[derive(Debug, Clone)]
    struct Echo;

    impl Task for Echo {
        type Input = String;
        type Output = String;
        type Error = std::convert::Infallible;

        async fn run(&self, input: Self::Input, _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
            Ok(input)
        }
    }

    impl SerializableTask for Echo {
        fn serializable_type_name(&self) -> &'static str {
            "test::Echo"
        }

        fn params(&self) -> JsonValue {
            serde_json::json!({})
        }
    }

    fn make_registry() -> TaskRegistry {
        let mut reg = TaskRegistry::new();
        reg.register_factory("test::Echo", |_params| Ok(erase(Echo)));
        reg
    }

    #[test]
    fn register_and_deserialize() {
        let reg = make_registry();
        let task = reg
            .deserialize("test::Echo", serde_json::json!({}))
            .unwrap();
        let _ = task.type_name();
    }

    #[test]
    fn unknown_type_returns_error() {
        let reg = make_registry();
        let result = reg.deserialize("test::Nope", serde_json::json!({}));
        assert!(matches!(result, Err(RegistryError::UnknownTaskType(_))));
    }

    #[test]
    fn contains_reflects_registration() {
        let mut reg = TaskRegistry::new();
        assert!(!reg.contains("test::Echo"));
        reg.register_factory("test::Echo", |_| Ok(erase(Echo)));
        assert!(reg.contains("test::Echo"));
    }

    #[test]
    fn snapshot_roundtrip() {
        let task = Echo;
        let snapshot = TaskSnapshot::from_task(&task);
        assert_eq!(snapshot.type_name, "test::Echo");
        assert!(snapshot.params.is_object());

        let reg = make_registry();
        let reconstructed = reg.deserialize_snapshot(&snapshot).unwrap();
        let _ = reconstructed.type_name();
    }

    #[test]
    fn snapshot_to_json_and_back() {
        let task = Echo;
        let snapshot = TaskSnapshot::from_task(&task);
        let json = snapshot.to_json().unwrap();
        // Just verify it's valid JSON containing the type name
        assert!(json.contains("test::Echo"));
    }

    #[test]
    fn registered_types_iter() {
        let mut reg = TaskRegistry::new();
        reg.register_factory("test::A", |_| Ok(erase(Echo)));
        reg.register_factory("test::B", |_| Ok(erase(Echo)));

        let mut types: Vec<_> = reg.registered_types().collect();
        types.sort();
        assert_eq!(types, ["test::A", "test::B"]);
    }
}
