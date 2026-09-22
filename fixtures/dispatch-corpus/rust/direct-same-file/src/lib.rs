pub fn target() -> i32 {
    42
}

#[test]
fn test_direct() {
    assert_eq!(target(), 42);
}
