//! Compile-time validation of the syscall DSL.
//!
//! Checks for duplicate numbers, offset range violations, and error positivity.

use std::collections::{HashMap, HashSet};

use crate::model::SyscallDefs;

/// Validate the parsed definitions and return compile errors if any.
pub(crate) fn validate(defs: &SyscallDefs) -> Result<(), Vec<syn::Error>> {
    let mut errors = Vec::new();

    // Check error values are positive.
    for err in &defs.errors {
        let val: isize = err
            .value
            .base10_parse()
            .expect("error value must be an integer");
        if val <= 0 {
            errors.push(syn::Error::new(
                err.name.span(),
                format!("error code `{}` must be positive, got {val}", err.name),
            ));
        }
    }

    // Check error names are unique.
    let mut error_names = HashSet::new();
    for err in &defs.errors {
        if !error_names.insert(err.name.to_string()) {
            errors.push(syn::Error::new(
                err.name.span(),
                format!("duplicate error name `{}`", err.name),
            ));
        }
    }

    // Check error values are unique.
    let mut error_values: HashMap<isize, &syn::Ident> = HashMap::new();
    for err in &defs.errors {
        let val: isize = err.value.base10_parse().unwrap();
        if let Some(prev) = error_values.insert(val, &err.name) {
            errors.push(syn::Error::new(
                err.name.span(),
                format!("error code value {val} already used by `{prev}`",),
            ));
        }
    }

    // Validate groups and syscalls.
    let mut all_numbers: HashMap<usize, (String, proc_macro2::Span)> = HashMap::new();

    for group in &defs.groups {
        // Check group range is valid.
        if group.range_start >= group.range_end {
            errors.push(syn::Error::new(
                group.name.span(),
                format!(
                    "group `{}`: range start ({:#x}) must be less than end ({:#x})",
                    group.name, group.range_start, group.range_end
                ),
            ));
        }

        let group_size = group.range_end - group.range_start;

        for syscall in &group.syscalls {
            // Check offset is within group range.
            if syscall.offset >= group_size {
                errors.push(syn::Error::new(
                    syscall.span,
                    format!(
                        "syscall `{}`: offset {:#x} exceeds group `{}` size ({:#x})",
                        syscall.name, syscall.offset, group.name, group_size
                    ),
                ));
            }

            // Check global uniqueness of syscall numbers.
            let number = syscall.number(group.range_start);
            if let Some((prev_name, _)) =
                all_numbers.insert(number, (syscall.name.to_string(), syscall.span))
            {
                errors.push(syn::Error::new(
                    syscall.span,
                    format!(
                        "syscall `{}` number {number:#x} collides with `{prev_name}`",
                        syscall.name
                    ),
                ));
            }

            // Check argument count (max 5 for x86_64 syscall ABI).
            if syscall.args.len() > 5 {
                errors.push(syn::Error::new(
                    syscall.span,
                    format!(
                        "syscall `{}` has {} arguments, max is 5",
                        syscall.name,
                        syscall.args.len()
                    ),
                ));
            }
        }
    }

    // Check group ranges don't overlap.
    let mut group_ranges: Vec<(usize, usize, String)> = defs
        .groups
        .iter()
        .map(|g| (g.range_start, g.range_end, g.name.to_string()))
        .collect();
    group_ranges.sort_by_key(|&(start, _, _)| start);

    for pair in group_ranges.windows(2) {
        let (_, end_a, ref name_a) = pair[0];
        let (start_b, _, ref name_b) = pair[1];
        if end_a > start_b {
            errors.push(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("group `{name_a}` range overlaps with group `{name_b}`"),
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
