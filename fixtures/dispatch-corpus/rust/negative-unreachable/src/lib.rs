pub fn target() -> i32 {
    42
}

pub fn unrelated() -> i32 {
    1
}

#[test]
fn test_unrelated() {
    assert_eq!(unrelated(), 1);
}
