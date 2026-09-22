pub fn target() -> i32 {
    42
}

pub fn run(target: fn() -> i32) -> i32 {
    target()
}

#[test]
fn test_shadowed() {
    assert_eq!(run(target), 42);
}
