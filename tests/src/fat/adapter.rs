use super::model::{FsState, Operation, format_trace, summarize_operation};

/// A mutable FAT implementation driven through the shared operation model.
///
/// Each adapter owns an already formatted image and is expected to reopen it
/// per operation so every step is durably committed to disk.
pub trait FatAdapter {
    fn apply(&mut self, operation: &Operation) -> Result<(), String>;
    fn snapshot(&mut self) -> Result<FsState, String>;
}

/// Applies a trace to an adapter and returns the model state it should now
/// match. Failures carry the trace prefix so a generated scenario can be
/// replayed.
pub fn apply_operations(
    adapter: &mut dyn FatAdapter,
    operations: &[Operation],
) -> Result<FsState, String> {
    let mut expected = FsState::empty();
    for (index, operation) in operations.iter().enumerate() {
        adapter.apply(operation).map_err(|error| {
            format!(
                "operation {index} failed: {}\n{error}\ntrace:\n{}",
                summarize_operation(operation),
                format_trace(&operations[..=index])
            )
        })?;
        expected.apply(operation).map_err(|error| {
            format!(
                "model rejected operation {index}: {}: {error}",
                summarize_operation(operation)
            )
        })?;
    }
    Ok(expected)
}

/// Like [`apply_operations`] for peers that cannot set attributes; the model
/// keeps default attributes so comparisons use [`clear_mutable_attrs`].
pub fn apply_operations_without_attrs(
    adapter: &mut dyn FatAdapter,
    operations: &[Operation],
) -> Result<FsState, String> {
    let mut expected = FsState::empty();
    for (index, operation) in operations.iter().enumerate() {
        adapter.apply(operation).map_err(|error| {
            format!(
                "operation {index} failed: {}\n{error}\ntrace:\n{}",
                summarize_operation(operation),
                format_trace(&operations[..=index])
            )
        })?;
        if !matches!(operation, Operation::SetAttrs { .. }) {
            expected.apply(operation)?;
        }
    }
    Ok(expected)
}

pub fn clear_mutable_attrs(state: &mut FsState) {
    for entry in state.entries.values_mut() {
        entry.attrs = 0;
    }
}
