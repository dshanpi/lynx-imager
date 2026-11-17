//! Reset-line updates must not reinstall a retired 0.9.x release.
pub fn compatible_line(current: (u64, u64), candidate: (u64, u64)) -> bool {
    current != (0, 1) || candidate == (0, 1)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reset_line_excludes_retired_and_unapproved_lines() {
        assert!(!compatible_line((0, 1), (0, 9)));
        assert!(!compatible_line((0, 1), (0, 7)));
        assert!(!compatible_line((0, 1), (0, 2)));
        assert!(!compatible_line((0, 1), (1, 0)));
    }
    #[test]
    fn reset_patches_and_legacy_behavior_are_retained() {
        assert!(compatible_line((0, 1), (0, 1)));
        assert!(compatible_line((0, 9), (0, 10)));
    }
}
