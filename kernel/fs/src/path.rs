//! Path parsing utilities for the VFS layer.
//!
//! Provides functions for splitting paths into components, checking if a path
//! is absolute, and matching paths against mount points.

/// Split a path into its components, filtering empty segments.
///
/// Leading and trailing slashes are ignored. Multiple consecutive slashes
/// are treated as a single separator.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(components("/usr/bin/ls").collect::<Vec<_>>(), ["usr", "bin", "ls"]);
/// assert_eq!(components("/").collect::<Vec<_>>(), Vec::<&str>::new());
/// ```
pub fn components(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|s| !s.is_empty())
}

/// Returns `true` if the path starts with `/`.
#[must_use]
pub fn is_absolute(path: &str) -> bool {
    path.starts_with('/')
}

/// Find the longest mount point that is a prefix of `path`.
///
/// Mount points are compared as path prefixes (i.e. `/dev` matches `/dev/null`
/// but not `/device`). The root mount `/` always matches if present.
pub fn longest_prefix_match<'a>(
    path: &str,
    mount_points: impl Iterator<Item = &'a str>,
) -> Option<&'a str> {
    let mut best: Option<&str> = None;

    for mp in mount_points {
        let matches = if mp == "/" {
            // Root mount matches everything.
            true
        } else {
            // Non-root mount: path must start with the mount point and either
            // end there or have a '/' immediately after.
            path == mp || path.starts_with(mp) && path.as_bytes().get(mp.len()) == Some(&b'/')
        };

        if matches && best.is_none_or(|b| mp.len() > b.len()) {
            best = Some(mp);
        }
    }

    best
}

/// Strip the mount prefix from a path, returning the remainder.
///
/// If the mount is `/`, the entire path is returned (without the leading `/`).
/// Otherwise, the mount prefix and its trailing slash are removed.
#[must_use]
pub fn strip_mount_prefix<'a>(path: &'a str, mount: &str) -> &'a str {
    if mount == "/" {
        path.strip_prefix('/').unwrap_or(path)
    } else if path.len() == mount.len() {
        ""
    } else {
        // Strip mount prefix + the '/' separator.
        &path[mount.len() + 1..]
    }
}
